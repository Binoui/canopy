//! Which live pages Canopy keeps: a memory budget and the least-recently-shown order.
//!
//! The policy is [`next_to_drop`], a pure function tests call without a display. [`measure`] and
//! [`Budget::of_machine`] read `/proc`; nothing here touches GTK or WebKit.
//!
//! What is counted is Canopy's whole process tree: this process, the chrome's and every page's
//! web process, the network processes and the sandbox helpers, PSS plus SwapPss. WebKit does not
//! say which web process serves which view, and it swaps processes on cross-site navigation, so
//! per-page attribution is not available. Swap is counted because a hidden page's memory moves to
//! swap after a minute or two without being freed (`page-cost-report.md`, item 4).

use anyhow::Result;

use crate::browser::ViewId;

/// How much memory Canopy may use before it drops live pages, in bytes: one eighth of the
/// machine's memory.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Budget(u64);

impl Budget {
    /// One eighth of `MemTotal` from `/proc/meminfo`.
    pub fn of_machine() -> Result<Self> {
        static CONFIGURED: std::sync::OnceLock<std::result::Result<Budget, String>> =
            std::sync::OnceLock::new();
        CONFIGURED
            .get_or_init(|| Self::read_configured().map_err(|e| e.to_string()))
            .clone()
            .map_err(anyhow::Error::msg)
    }

    fn read_configured() -> Result<Self> {
        if let Ok(value) = std::env::var("CANOPY_MEMORY_BUDGET_MB") {
            return Ok(Self(
                value
                    .parse::<u64>()?
                    .checked_mul(1 << 20)
                    .ok_or_else(|| anyhow::anyhow!("Memory budget overflow"))?,
            ));
        }
        let info = std::fs::read_to_string("/proc/meminfo")?;
        let total = info
            .lines()
            .find_map(|line| line.strip_prefix("MemTotal:"))
            .ok_or_else(|| anyhow::anyhow!("Missing MemTotal"))?;
        Ok(Self::of_total(
            total
                .split_whitespace()
                .next()
                .unwrap_or("")
                .parse::<u64>()?
                * 1024,
        ))
    }

    /// One eighth of `total` bytes.
    pub fn of_total(total: u64) -> Self {
        Self(total / 8)
    }

    pub fn bytes(self) -> u64 {
        self.0
    }
}

/// A live page the policy may drop.
///
/// The caller lists only hidden pages that have committed a load and are not being opened. The
/// page on screen and the page being opened are never in the list, so they cannot be dropped
/// whatever the budget says.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Droppable {
    pub page: ViewId,
    /// When the page was last shown or asked for, on the window's clock: higher is more recent.
    pub last_shown: u64,
    /// WebKit reports the page playing audio. Read from the webview at each decision, never
    /// cached, so the exemption ends when the sound does.
    pub audible: bool,
}

/// The page to drop now, or `None` when `used` fits the budget or every candidate plays audio.
///
/// One page at a time: a dropped page's memory returns only once its web process exits, about a
/// second later, so the caller measures again before dropping another.
pub fn next_to_drop(used: u64, budget: Budget, pages: &[Droppable]) -> Option<ViewId> {
    if used <= budget.bytes() {
        return None;
    }
    pages
        .iter()
        .filter(|page| !page.audible)
        .min_by_key(|page| page.last_shown)
        .map(|page| page.page)
}

/// PSS plus SwapPss of this process and all its descendants, in bytes, from each process's
/// `/proc/<pid>/smaps_rollup`. A process that exits while being read is skipped.
pub fn measure() -> Result<u64> {
    let mut pending = vec![std::process::id()];
    let mut seen = std::collections::HashSet::new();
    let mut used = 0;
    while let Some(pid) = pending.pop() {
        if !seen.insert(pid) {
            continue;
        }
        let read = || -> Result<(u64, Vec<u32>)> {
            let tasks = std::fs::read_dir(format!("/proc/{pid}/task"))?;
            let mut children = Vec::new();
            for task in tasks {
                let task = task?;
                match std::fs::read_to_string(task.path().join("children")) {
                    Ok(text) => {
                        for child in text.split_whitespace() {
                            children.push(child.parse()?);
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e.into()),
                }
            }
            let rollup = std::fs::read_to_string(format!("/proc/{pid}/smaps_rollup"))?;
            let mut total = 0;
            let mut fields = 0;
            for line in rollup.lines() {
                if line.starts_with("Pss:") || line.starts_with("SwapPss:") {
                    total += line
                        .split_whitespace()
                        .nth(1)
                        .unwrap_or("")
                        .parse::<u64>()?
                        * 1024;
                    fields += 1;
                }
            }
            anyhow::ensure!(fields == 2, "Incomplete smaps_rollup for {pid}");
            Ok((total, children))
        };
        match read() {
            Ok((bytes, children)) => {
                used += bytes;
                pending.extend(children);
            }
            Err(error)
                if error.downcast_ref::<std::io::Error>().is_some_and(|e| {
                    e.kind() == std::io::ErrorKind::NotFound || e.raw_os_error() == Some(3)
                }) => {}
            Err(error) => return Err(error),
        }
    }
    Ok(used)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIB: u64 = 1 << 20;

    fn page(last_shown: u64, audible: bool) -> Droppable {
        Droppable {
            page: crate::browser::Navigation::default().view_id(),
            last_shown,
            audible,
        }
    }

    #[test]
    fn nothing_is_dropped_within_the_budget() {
        let budget = Budget::of_total(32 * 1024 * MIB);
        assert_eq!(budget.bytes(), 4 * 1024 * MIB);
        let pages = [page(1, false), page(2, false)];
        assert_eq!(next_to_drop(budget.bytes(), budget, &pages), None);
        assert_eq!(next_to_drop(budget.bytes() - 1, budget, &pages), None);
        assert_eq!(next_to_drop(u64::MAX, budget, &[]), None);
        let ties = [page(1, false), page(1, false)];
        assert_eq!(next_to_drop(u64::MAX, budget, &ties), Some(ties[0].page));
    }

    #[test]
    fn over_the_budget_the_least_recently_shown_page_goes_first() {
        let budget = Budget::of_total(8 * 1024 * MIB);
        let pages = [page(7, false), page(3, false), page(5, false)];
        assert_eq!(
            next_to_drop(budget.bytes() + 1, budget, &pages),
            Some(pages[1].page)
        );
    }

    #[test]
    fn a_page_playing_audio_is_never_dropped() {
        let budget = Budget::of_total(8 * 1024 * MIB);
        let pages = [page(1, true), page(4, false)];
        assert_eq!(
            next_to_drop(budget.bytes() * 2, budget, &pages),
            Some(pages[1].page)
        );
        assert_eq!(next_to_drop(budget.bytes() * 2, budget, &pages[..1]), None);
    }
}
