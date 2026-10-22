use gpui::InteractiveElement;

/// Apply only to the header title and empty space, excluding toolbar buttons.
pub(super) trait WindowDragArea: InteractiveElement {
    fn window_drag_area(self) -> Self {
        #[cfg(target_os = "windows")]
        {
            self.window_control_area(gpui::WindowControlArea::Drag)
        }

        #[cfg(not(target_os = "windows"))]
        {
            self.on_mouse_down(gpui::MouseButton::Left, |event, window, cx| {
                if cfg!(target_os = "macos") && event.click_count == 2 {
                    window.titlebar_double_click();
                } else {
                    // On Wayland this must run during the mouse press so GPUI
                    // can pass its serial to the compositor. macOS uses AppKit.
                    window.start_window_move();
                }
                cx.stop_propagation();
            })
        }
    }
}

impl<E: InteractiveElement> WindowDragArea for E {}
