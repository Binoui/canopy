use super::*;
use std::time::{Duration, Instant};
fn sample() -> Result<(u64, f64)> {
    thread::spawn(|| {
        let start = Instant::now();
        canopy::budget::measure().map(|used| (used, start.elapsed().as_secs_f64() * 1000.))
    })
    .join()
    .unwrap()
}
fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    (values[9] + values[10]) / 2.
}
fn colour(f: &Fixture) -> [u8; 3] {
    let pix =
        f.ui.window
            .window()
            .unwrap()
            .pixbuf(400, 400, 1, 1)
            .unwrap();
    let bytes = pix.read_pixel_bytes();
    [bytes[0], bytes[1], bytes[2]]
}
fn first_frame(f: &Fixture, target: canopy::browser::VisitId, rgb: [u8; 3]) -> Result<f64> {
    let start = Instant::now();
    f.ui.pages.go(Go::Visit(target))?;
    let old = colour(f);
    let unexpected = std::cell::Cell::new(false);
    common::wait("target pixels shown", || {
        let pixel = colour(f);
        if pixel != old && pixel != rgb {
            unexpected.set(true);
        }
        pixel == rgb
    })?;
    assert!(!unexpected.get(), "switch exposed pixels from neither page");
    Ok(start.elapsed().as_secs_f64() * 1000.)
}
fn cpu_ticks() -> Result<u64> {
    let stat = std::fs::read_to_string("/proc/self/stat")?;
    let fields: Vec<_> = stat
        .rsplit_once(')')
        .unwrap()
        .1
        .split_whitespace()
        .collect();
    Ok(fields[11].parse::<u64>()? + fields[12].parse::<u64>()?)
}
fn processes(pid: u32) -> Result<()> {
    println!(
        "DESCENDANT pid={pid} name={}",
        std::fs::read_to_string(format!("/proc/{pid}/comm"))?.trim()
    );
    for task in std::fs::read_dir(format!("/proc/{pid}/task"))? {
        let children = std::fs::read_to_string(task?.path().join("children"))?;
        for pid in children.split_whitespace() {
            processes(pid.parse()?)?;
        }
    }
    Ok(())
}
pub(super) fn run() -> Result<()> {
    if std::env::var_os("CANOPY_MEASURE_CPU_ONLY").is_none() {
        {
            let f = Fixture::new()?;
            let red = f.root("/red")?;
            let blue = f.root("/blue")?;
            common::pump(1200);
            let mut live = Vec::new();
            for _ in 0..20 {
                first_frame(&f, blue, [0, 0, 255])?;
                live.push(first_frame(&f, red, [255, 0, 0])?);
            }
            println!(
                "MEASURE live_input_to_pixels_ms_median20={:.3}",
                median(live)
            );
            if std::env::var_os("CANOPY_MEASURE_FRAMES_ONLY").is_some() {
                return Ok(());
            }
            let mut dormant = Vec::new();
            for _ in 0..20 {
                first_frame(&f, blue, [0, 0, 255])?;
                common::pump(1300);
                f.ui.pages.inject_memory_sample(Some(u64::MAX));
                common::wait("red dropped", || !f.ui.pages.live_visits().contains(&red))?;
                common::pump(600);
                dormant.push(first_frame(&f, red, [255, 0, 0])?);
            }
            println!(
                "MEASURE dormant_input_to_pixels_ms_median20={:.3}",
                median(dormant)
            );
            if std::env::var_os("CANOPY_MEASURE_SWITCHES_ONLY").is_some() {
                return Ok(());
            }
        }
        {
            let f = Fixture::new()?;
            let plain = f.root("/plain0")?;
            common::pump(1300);
            let initial = sample()?.0;
            f.root("/plain1")?;
            common::pump(1300);
            let extra = sample()?.0;
            println!(
                "MEASURE plain_extra_live_bytes={} before={initial} after={extra}",
                extra as i64 - initial as i64
            );
            f.root("/heavy")?;
            common::pump(1300);
            let heavy = f.ui.snapshot()?.current.unwrap();
            let high = sample()?.0;
            f.select(plain)?;
            common::pump(1300);
            // Drop the older plain page first, then the heavy page, each with a fresh sample.
            f.ui.pages.inject_memory_sample(Some(u64::MAX));
            common::pump(1800);
            f.ui.pages.inject_memory_sample(Some(u64::MAX));
            common::wait("heavy dropped", || {
                !f.ui.pages.live_visits().contains(&heavy)
            })?;
            common::pump(1800);
            let low = sample()?.0;
            println!(
                "MEASURE sandbox={} heavy_before={extra} heavy_open={high} after_drop={low}",
                f.ui.pages.sandbox_enabled()
            );
            processes(std::process::id())?;
        }
        {
            let f = Fixture::new()?;
            for i in 0..10 {
                f.root(&format!("/plain{i}"))?;
                common::pump(1200);
                if [1, 5, 10].contains(&(i + 1)) {
                    assert_eq!(f.ui.pages.live_visits().len(), i + 1);
                    let mut times = Vec::new();
                    for _ in 0..20 {
                        times.push(sample()?.1);
                    }
                    println!(
                        "MEASURE sample_ms_live_pages_{}_median20={:.3}",
                        i + 1,
                        median(times)
                    );
                }
            }
        }
    }
    {
        let f = Fixture::new()?;
        for i in 0..6 {
            f.root(&format!("/plain{i}"))?;
        }
        common::pump(2000);
        let ticks_per_second: f64 = String::from_utf8(
            std::process::Command::new("getconf")
                .arg("CLK_TCK")
                .output()?
                .stdout,
        )?
        .trim()
        .parse()?;
        let before = cpu_ticks()?;
        let start = Instant::now();
        glib::timeout_add_local_once(Duration::from_secs(60), gtk::main_quit);
        gtk::main();
        assert!(start.elapsed() >= Duration::from_secs(60));
        let cpu = (cpu_ticks()? - before) as f64 / ticks_per_second;
        println!(
            "MEASURE process_cpu_seconds_idle_5_hidden_60s={cpu:.3} percent_one_core={:.3}",
            cpu / 0.6
        );
    }
    Ok(())
}
