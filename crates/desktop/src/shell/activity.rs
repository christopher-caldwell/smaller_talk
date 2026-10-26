use super::*;
use gpui::{Animation, AnimationExt};

pub(super) struct ActivityIndicator {
    activity: Activity,
    color: u32,
    small: bool,
    #[cfg(test)]
    renders: usize,
}
impl Render for ActivityIndicator {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        #[cfg(test)]
        {
            self.renders += 1;
        }
        let waiting = self.activity == Activity::Waiting;
        let dot = div()
            .size(px(6.))
            .flex_shrink_0()
            .rounded_full()
            .bg(rgb(if waiting { ATTENTION } else { self.color }));
        let dot = if waiting {
            dot.into_any_element()
        } else {
            dot.with_animation(
                "activity-pulse",
                Animation::new(Duration::from_millis(1200))
                    .repeat_synced()
                    .with_max_fps(20.),
                |dot, phase| {
                    dot.opacity(0.4 + 0.6 * ((phase * std::f32::consts::TAU).cos() + 1.) / 2.)
                },
            )
            .into_any_element()
        };
        row()
            .text_size(px(if self.small { 11. } else { 13. }))
            .text_color(rgb(self.color))
            .child(dot)
            .child(self.activity.label())
    }
}
impl ChatsView {
    pub(super) fn render_activity(
        &mut self,
        id: Uuid,
        activity: Activity,
        color: u32,
        small: bool,
        cx: &mut Context<Self>,
    ) -> Entity<ActivityIndicator> {
        let entity = self
            .activity_indicators
            .entry((id, small))
            .or_insert_with(|| {
                cx.new(|_| ActivityIndicator {
                    activity,
                    color,
                    small,
                    #[cfg(test)]
                    renders: 0,
                })
            })
            .clone();
        entity.update(cx, |indicator, cx| {
            if indicator.activity != activity || indicator.color != color {
                indicator.activity = activity;
                indicator.color = color;
                cx.notify();
            }
        });
        entity
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[gpui::test]
    fn pulse_stops_for_waiting_and_reduced_motion(cx: &mut gpui::TestAppContext) {
        let window = cx.open_window(gpui::size(px(160.), px(30.)), |_, _| ActivityIndicator {
            activity: Activity::Working,
            color: ACCENT,
            small: false,
            renders: 0,
        });
        cx.run_until_parked();
        let initial = cx.read(|cx| window.read(cx).unwrap().renders);
        cx.executor().advance_clock(Duration::from_millis(120));
        cx.run_until_parked();
        assert!(cx.read(|cx| window.read(cx).unwrap().renders) > initial);
        window
            .update(cx, |view, _, cx| {
                view.activity = Activity::Waiting;
                cx.notify();
            })
            .unwrap();
        cx.run_until_parked();
        // The previous mounted animation can have one already-scheduled frame.
        // Drain it, then prove the static indicator schedules no further work.
        cx.executor().advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
        let waiting = cx.read(|cx| window.read(cx).unwrap().renders);
        cx.executor().advance_clock(Duration::from_secs(2));
        cx.run_until_parked();
        assert_eq!(cx.read(|cx| window.read(cx).unwrap().renders), waiting);
        cx.update(|cx| cx.set_reduce_motion(true));
        window
            .update(cx, |view, _, cx| {
                view.activity = Activity::Working;
                cx.notify();
            })
            .unwrap();
        cx.run_until_parked();
        let reduced = cx.read(|cx| window.read(cx).unwrap().renders);
        cx.executor().advance_clock(Duration::from_secs(2));
        cx.run_until_parked();
        assert_eq!(cx.read(|cx| window.read(cx).unwrap().renders), reduced);
    }
}
