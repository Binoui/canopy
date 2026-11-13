use std::time::Duration;

/// Pointer and modal state of the chrome, as a pure state machine. The GTK layer feeds it
/// crossings, timer expiries and modal changes, and carries out the outputs.
#[derive(Debug, Default)]
pub(crate) struct Presentation {
    open: bool,
    layer: Layer,
    tokens: [u64; 2],
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
}

impl Timer {
    fn delay(self) -> Duration {
        Duration::from_millis(match self {
            Timer::Open => 110,
            Timer::Close => 380,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Input {
    EnterStrip,
    LeaveStrip,
    Fired(Timer, u64),
    Layer(Layer),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Output {
    Schedule(Timer, Duration, u64),
    Strip { open: bool },
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
                    out.push(self.schedule(Timer::Open));
                }
            }
            Input::LeaveStrip => {
                self.cancel(Timer::Open);
                if self.open {
                    out.push(self.schedule(Timer::Close));
                }
            }
            Input::Fired(timer, token) if token == self.token(timer) => match timer {
                Timer::Open if !self.open && self.layer == Layer::None => {
                    self.open = true;
                    out.push(Output::Strip { open: true });
                }
                Timer::Close if self.open && self.layer != Layer::Menu => {
                    self.open = false;
                    out.push(Output::Strip { open: false });
                }
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
                    }
                    Layer::Menu => self.cancel(Timer::Close),
                    Layer::None if previous == Layer::Menu && self.open => {
                        out.push(self.schedule(Timer::Close));
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

    fn schedule(&mut self, timer: Timer) -> Output {
        self.cancel(timer);
        Output::Schedule(timer, timer.delay(), self.token(timer))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
