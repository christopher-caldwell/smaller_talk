use gpui::{
    AnyElement, ElementId, IntoElement, ParentElement, Stateful, Styled, div, prelude::*, px, rgb,
};

pub const BACKGROUND: u32 = 0x191c1f;
pub const PANEL: u32 = 0x242629;
pub const PANEL_HOVER: u32 = 0x36363d;
pub const NAVIGATION: u32 = 0x201a36;
pub const NAV_HOVER: u32 = 0x34294d;
pub const NAV_SELECTED: u32 = 0x73cc8f;
pub const SELECTED_INK: u32 = 0x14251b;
pub const CHROME: u32 = 0x302360;
pub const COMPOSER: u32 = 0x303134;
pub const MENU: u32 = 0x242427;
pub const ACCENT: u32 = 0x92cda4;
pub const FOCUS: u32 = 0xa6a6ad;
pub const TEXT: u32 = 0xf1f0f4;
pub const MUTED: u32 = 0xb4afbe;
pub const DANGER: u32 = 0xf39caa;
pub const BORDER: u32 = 0x414047;
pub const SEND: u32 = 0xf4f3f7;

pub fn button(id: impl Into<ElementId>, label: impl Into<String>) -> Stateful<gpui::Div> {
    button_with_hover(id, label, PANEL_HOVER)
}
pub fn button_with_hover(
    id: impl Into<ElementId>,
    label: impl Into<String>,
    hover: u32,
) -> Stateful<gpui::Div> {
    let label = label.into();
    div()
        .id(id)
        .tab_index(0)
        .border_1()
        .border_color(gpui::rgba(0))
        .focus(|s| s.border_color(rgb(FOCUS)).bg(rgb(PANEL_HOVER)))
        .px_3()
        .py_2()
        .rounded_md()
        .text_sm()
        .text_color(rgb(TEXT))
        .cursor_pointer()
        .hover(move |s| s.bg(rgb(hover)))
        .when(!label.is_empty(), |button| button.child(label))
}
pub fn muted(text: impl Into<String>) -> AnyElement {
    div()
        .text_sm()
        .text_color(rgb(MUTED))
        .child(text.into())
        .into_any_element()
}
pub fn row() -> gpui::Div {
    div().flex().items_center().gap_2().min_w(px(0.))
}

pub fn thumbnail(
    id: impl Into<ElementId>,
    path: std::path::PathBuf,
    width: f32,
    height: f32,
) -> AnyElement {
    // Scope decoded textures to this mounted thumbnail instead of the global asset cache.
    gpui::image_cache(gpui::retain_all(id))
        .child(
            gpui::img(path)
                .w(px(width))
                .h(px(height))
                .object_fit(gpui::ObjectFit::Contain),
        )
        .into_any_element()
}

struct Hint {
    text: String,
    visible: bool,
    _delay: gpui::Task<()>,
}
impl Hint {
    fn new(text: String, cx: &mut gpui::Context<Self>) -> Self {
        // GPUI waits 500 ms before building a tooltip. Keep it empty for
        // another second; dropping the tooltip cancels this view-owned task.
        let delay = cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(1000))
                .await;
            let _ = this.update(cx, |this, cx| {
                this.visible = true;
                cx.notify();
            });
        });
        Self {
            text,
            visible: false,
            _delay: delay,
        }
    }
}
impl gpui::Render for Hint {
    fn render(&mut self, _: &mut gpui::Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        div().when(self.visible, |view| {
            view.max_w(px(300.))
                .px_3()
                .py_2()
                .rounded_md()
                .border_1()
                .border_color(rgb(BORDER))
                .bg(rgb(PANEL))
                .text_color(rgb(TEXT))
                .text_sm()
                .shadow_lg()
                .child(self.text.clone())
        })
    }
}

pub fn hint(
    text: impl Into<String>,
) -> impl Fn(&mut gpui::Window, &mut gpui::App) -> gpui::AnyView {
    let text = text.into();
    move |_, cx| {
        use gpui::AppContext as _;
        cx.new(|cx| Hint::new(text.clone(), cx)).into()
    }
}

// Keep vertical gestures in the transcript; wide blocks own only their horizontal axis.
pub fn horizontal_scroller(
    id: impl Into<ElementId>,
    scroll: &gpui::ScrollHandle,
) -> Stateful<gpui::Div> {
    let handle = scroll.clone();
    let mut view = div()
        .id(id)
        .tab_index(0)
        .w_full()
        .min_w(px(0.))
        .overflow_x_scroll()
        .track_scroll(scroll)
        .focus(|s| s.bg(rgb(PANEL_HOVER)))
        .tooltip(hint("Scroll horizontally; use Left/Right when focused"))
        .on_key_down(move |event, window, cx| {
            let mut offset = handle.offset();
            match event.keystroke.key.as_str() {
                "left" => offset.x += px(40.),
                "right" => offset.x -= px(40.),
                "home" => offset.x = px(0.),
                "end" => offset.x = -handle.max_offset().width,
                _ => return,
            }
            handle.set_offset(offset);
            window.prevent_default();
            cx.stop_propagation();
            window.refresh();
        });
    view.style().restrict_scroll_to_axis = Some(true);
    view
}

pub struct Icons;
impl gpui::AssetSource for Icons {
    fn load(&self, path: &str) -> anyhow::Result<Option<std::borrow::Cow<'static, [u8]>>> {
        let bytes: &'static [u8] = match path {
            "pencil" => include_bytes!("../assets/icons/pencil.svg"),
            "folder" => include_bytes!("../assets/icons/folder.svg"),
            "trash" => include_bytes!("../assets/icons/trash.svg"),
            "chevron" => include_bytes!("../assets/icons/chevron.svg"),
            "compose" => include_bytes!("../assets/icons/compose.svg"),
            "queue" => include_bytes!("../assets/icons/queue.svg"),
            "copy" => include_bytes!("../assets/icons/copy.svg"),
            _ => return Ok(None),
        };
        Ok(Some(std::borrow::Cow::Borrowed(bytes)))
    }
    fn list(&self, _: &str) -> anyhow::Result<Vec<gpui::SharedString>> {
        Ok(Vec::new())
    }
}
pub fn icon(name: &'static str) -> gpui::Svg {
    gpui::svg()
        .path(name)
        .size(px(16.))
        .flex_shrink_0()
        .text_color(rgb(TEXT))
}
pub fn menu_item(id: impl Into<ElementId>, name: &'static str, label: &str) -> Stateful<gpui::Div> {
    button(id, "")
        .w_full()
        .flex()
        .items_center()
        .gap_3()
        .py_1()
        .px_2()
        .child(icon(name))
        .child(label.to_owned())
}
