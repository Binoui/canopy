use std::time::Duration;

use crate::browser::VisitId;

/// Pointer and modal state of the chrome, as a pure state machine. The GTK layer feeds it
/// crossings, timer expiries and modal changes, and carries out the outputs.
#[derive(Debug, Default)]
pub(crate) struct Presentation {
    open: bool,
    layer: Layer,
    /// The page the pointer rests on, whose card shows once its timer fires.
    hovered: Option<VisitId>,
    card: Option<VisitId>,
    tokens: [u64; 4],
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum Layer {
    #[default]
    None,
    /// A popover anchored in the strip, such as the Memory menu or a "+N" list.
    Menu,
    Palette,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Timer {
    Open,
    Close,
    ShowCard,
    HideCard,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Input {
    EnterStrip,
    LeaveStrip,
    EnterNode(VisitId),
    LeaveNode,
    EnterCard,
    LeaveCard,
    /// A node or card action ran.
    Activated,
    Fired(Timer, u64),
    Layer(Layer),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Output {
    Schedule(Timer, Duration, u64),
    Strip { open: bool },
    ShowCard(VisitId),
    HideCard,
}

impl Presentation {
    pub fn layer(&self) -> Layer {
        self.layer
    }

    pub fn step(&mut self, input: Input) -> Vec<Output> {
        let mut out = Vec::new();
        match input {
            Input::EnterStrip if self.layer != Layer::Palette => {
                self.cancel(Timer::Close);
                if !self.open {
                    out.push(self.schedule(Timer::Open, 110));
                }
            }
            Input::LeaveStrip => {
                self.cancel(Timer::Open);
                if self.open {
                    out.push(self.schedule(Timer::Close, 380));
                }
            }
            Input::EnterNode(id) if self.layer == Layer::None => {
                self.hovered = Some(id);
                self.cancel(Timer::HideCard);
                let delay = if self.card.is_some() { 40 } else { 260 };
                out.push(self.schedule(Timer::ShowCard, delay));
            }
            Input::LeaveNode => {
                self.hovered = None;
                self.cancel(Timer::ShowCard);
                if self.card.is_some() {
                    out.push(self.schedule(Timer::HideCard, 160));
                }
            }
            Input::EnterCard => {
                self.cancel(Timer::HideCard);
                self.cancel(Timer::Close);
            }
            Input::LeaveCard => {
                if self.card.is_some() {
                    out.push(self.schedule(Timer::HideCard, 120));
                }
                if self.open {
                    out.push(self.schedule(Timer::Close, 380));
                }
            }
            Input::Activated => self.hide_card(&mut out),
            Input::Fired(timer, token) if token == self.token(timer) => match timer {
                Timer::Open if !self.open && self.layer == Layer::None => {
                    self.open = true;
                    out.push(Output::Strip { open: true });
                }
                Timer::Close if self.open && self.layer != Layer::Menu => {
                    self.open = false;
                    out.push(Output::Strip { open: false });
                    self.hide_card(&mut out);
                }
                Timer::ShowCard if self.open && self.layer == Layer::None => {
                    if let Some(id) = self.hovered.filter(|id| self.card != Some(*id)) {
                        self.card = Some(id);
                        out.push(Output::ShowCard(id));
                    }
                }
                Timer::HideCard => self.hide_card(&mut out),
                _ => {}
            },
            Input::Layer(layer) => {
                let previous = std::mem::replace(&mut self.layer, layer);
                match layer {
                    Layer::Palette => {
                        self.cancel(Timer::Open);
                        self.cancel(Timer::Close);
                        if self.open {
                            self.open = false;
                            out.push(Output::Strip { open: false });
                        }
                        self.hide_card(&mut out);
                    }
                    Layer::Menu => {
                        self.cancel(Timer::Close);
                        self.hide_card(&mut out);
                    }
                    Layer::None if previous == Layer::Menu && self.open => {
                        out.push(self.schedule(Timer::Close, 380));
                    }
                    Layer::None => {}
                }
            }
            _ => {}
        }
        out
    }

    fn token(&self, timer: Timer) -> u64 {
        self.tokens[timer as usize]
    }

    fn cancel(&mut self, timer: Timer) {
        self.tokens[timer as usize] += 1;
    }

    fn schedule(&mut self, timer: Timer, ms: u64) -> Output {
        self.cancel(timer);
        Output::Schedule(timer, Duration::from_millis(ms), self.token(timer))
    }

    fn hide_card(&mut self, out: &mut Vec<Output>) {
        self.cancel(Timer::ShowCard);
        self.cancel(Timer::HideCard);
        if self.card.take().is_some() {
            out.push(Output::HideCard);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn visit() -> VisitId {
        ulid::Ulid::new().to_string().parse().unwrap()
    }

    fn opened() -> Presentation {
        let mut p = Presentation::default();
        let out = p.step(Input::EnterStrip);
        fire(&mut p, &out);
        p
    }

    fn fire(p: &mut Presentation, out: &[Output]) -> Vec<Output> {
        out.iter()
            .filter_map(|o| match o {
                Output::Schedule(timer, _, token) => Some(p.step(Input::Fired(*timer, *token))),
                _ => None,
            })
            .flatten()
            .collect()
    }

    #[test]
    fn hover_opens_after_110_ms_and_leaving_closes_after_380_ms() {
        let mut p = Presentation::default();
        let out = p.step(Input::EnterStrip);
        assert_eq!(
            out,
            [Output::Schedule(Timer::Open, Duration::from_millis(110), 1)]
        );
        assert_eq!(fire(&mut p, &out), [Output::Strip { open: true }]);
        let out = p.step(Input::LeaveStrip);
        assert!(matches!(out[..], [Output::Schedule(Timer::Close, d, _)] if d.as_millis() == 380));
        assert_eq!(fire(&mut p, &out), [Output::Strip { open: false }]);
    }

    #[test]
    fn a_quick_pass_over_the_strip_never_opens_it() {
        let mut p = Presentation::default();
        let open = p.step(Input::EnterStrip);
        p.step(Input::LeaveStrip);
        assert!(fire(&mut p, &open).is_empty());
    }

    #[test]
    fn coming_back_before_the_close_timer_keeps_the_strip_open() {
        let mut p = Presentation::default();
        let out = p.step(Input::EnterStrip);
        fire(&mut p, &out);
        let close = p.step(Input::LeaveStrip);
        assert!(p.step(Input::EnterStrip).is_empty());
        assert!(fire(&mut p, &close).is_empty());
    }

    #[test]
    fn the_palette_closes_the_strip_and_ignores_hover() {
        let mut p = Presentation::default();
        let out = p.step(Input::EnterStrip);
        fire(&mut p, &out);
        assert_eq!(
            p.step(Input::Layer(Layer::Palette)),
            [Output::Strip { open: false }]
        );
        assert!(p.step(Input::EnterStrip).is_empty());
        p.step(Input::Layer(Layer::None));
        let out = p.step(Input::EnterStrip);
        assert_eq!(fire(&mut p, &out), [Output::Strip { open: true }]);
    }

    #[test]
    fn a_menu_holds_the_strip_open_until_it_closes() {
        let mut p = Presentation::default();
        let out = p.step(Input::EnterStrip);
        fire(&mut p, &out);
        p.step(Input::Layer(Layer::Menu));
        let close = p.step(Input::LeaveStrip);
        assert!(fire(&mut p, &close).is_empty());
        let close = p.step(Input::Layer(Layer::None));
        assert_eq!(fire(&mut p, &close), [Output::Strip { open: false }]);
    }

    #[test]
    fn a_card_follows_the_hovered_node_after_260_ms_then_40_ms() {
        let mut p = opened();
        let (a, b) = (visit(), visit());
        let out = p.step(Input::EnterNode(a));
        assert!(
            matches!(out[..], [Output::Schedule(Timer::ShowCard, d, _)] if d.as_millis() == 260)
        );
        assert_eq!(fire(&mut p, &out), [Output::ShowCard(a)]);
        p.step(Input::LeaveNode);
        let out = p.step(Input::EnterNode(b));
        assert!(
            matches!(out[..], [Output::Schedule(Timer::ShowCard, d, _)] if d.as_millis() == 40)
        );
        assert_eq!(fire(&mut p, &out), [Output::ShowCard(b)]);
    }

    #[test]
    fn moving_onto_the_card_keeps_it_and_the_strip() {
        let mut p = opened();
        let a = visit();
        let out = p.step(Input::EnterNode(a));
        fire(&mut p, &out);
        let hide = p.step(Input::LeaveNode);
        let close = p.step(Input::LeaveStrip);
        p.step(Input::EnterCard);
        assert!(fire(&mut p, &hide).is_empty());
        assert!(fire(&mut p, &close).is_empty());
        let out = p.step(Input::LeaveCard);
        let fired = fire(&mut p, &out);
        assert!(fired.contains(&Output::HideCard));
        assert!(fired.contains(&Output::Strip { open: false }));
    }

    #[test]
    fn no_card_while_resting_or_under_the_palette() {
        let mut p = Presentation::default();
        let out = p.step(Input::EnterNode(visit()));
        assert!(fire(&mut p, &out).is_empty());
        let mut p = opened();
        let out = p.step(Input::EnterNode(visit()));
        p.step(Input::Layer(Layer::Palette));
        assert!(fire(&mut p, &out).is_empty());
    }
}
