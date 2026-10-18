//! Keeps the custom (undecorated) title bar window truly borderless (#2246).
//!
//! winit 0.30 gives an undecorated window the `WS_CAPTION | WS_BORDER` styles and hides the
//! non-client area by spreading the client rect over it (`WM_NCCALCSIZE`). On some systems
//! (reported: Windows 10 LTSB, Intel HD Graphics 4600, 1366 × 768) the desktop compositor still
//! reserves that frame for the window surface: the content is drawn offset by the caption and
//! border, leaving black bars at the window's top-left, while the pointer keeps the un-offset
//! client coordinates, so it no longer points at what is drawn under it.
//!
//! winit 0.27 — before rust-windowing/winit#2419 — simply omitted those styles for undecorated
//! windows, and there was no offset. [`service`] restores that: it strips `WS_CAPTION` and
//! `WS_BORDER` and forces Windows to recalculate the frame. winit puts them back whenever it
//! changes the window flags (maximize, restore, …), so the returned guard checks every frame.
//!
//! Windows sizes a maximize by the frame its styles describe, so a window maximized while its
//! caption is gone is sized without one: its rect covers the whole monitor, taskbar included.
//! The shell reads a caption-less window covering its monitor as a full-screen app and drops the
//! taskbar's backdrop behind it — a black band on stock Windows 11, a glitched unresponsive one
//! under ExplorerPatcher (#3144). The guard therefore also trims a parked maximize's rect back to
//! the work area plus its own frame, clear of the taskbar. winit pins the client area to the work
//! area either way (`WM_NCCALCSIZE`), so the app's pixels do not move.
//!
//! An auto-hiding taskbar reserves no work area, so a maximize still covers the whole monitor and
//! the shell, reading it as a full-screen app, never slides the taskbar up. On each edge with such
//! a taskbar the guard stops the window's rect (its invisible sizing frame included: a rect that
//! still reaches past the edge counts as covering it) one pixel short of the monitor, as other
//! custom-frame apps do, so the pointer at that edge reveals the taskbar again. Measured on a
//! 2560 × 1440 monitor with a bottom auto-hide taskbar: a maximize parked at
//! `(-8, -8, 2568, 1447)` kept the taskbar down, `(-8, -8, 2568, 1439)` lets it up.
//!
//! The Windows calls go through safe `winsafe` wrappers; the workspace forbids `unsafe`, and
//! winsafe can't wrap a raw handle without it, so the window is found by its handle instead.

pub use platform::service;

#[cfg(target_os = "windows")]
mod platform {
    use eframe::wgpu::rwh::{HasWindowHandle, RawWindowHandle};
    use photocraft_ui_egui::WindowFrameFn;
    use winsafe::{HWND, HwndPlace, POINT, SIZE, co};

    /// A guard that keeps the window borderless each frame, or `None` when the app uses the
    /// system title bar (the window is decorated) or its handle can't be found.
    pub fn service(cc: &eframe::CreationContext<'_>, custom_titlebar: bool) -> Option<WindowFrameFn> {
        if !custom_titlebar {
            return None;
        }
        let target = match cc.window_handle().ok()?.as_raw() {
            RawWindowHandle::Win32(h) => h.hwnd.get(),
            _ => return None,
        };
        let hwnd = find_window(target)?;
        reapply(&hwnd);
        Some(Box::new(move || reapply(&hwnd)))
    }

    /// The top-level window with the given raw handle. `EnumWindows` hands out real `HWND`s, so
    /// this matches winit's own window without wrapping a raw handle (which needs `unsafe`).
    fn find_window(target: isize) -> Option<HWND> {
        let mut found = None;
        let _ = winsafe::EnumWindows(|hwnd| {
            if hwnd.ptr() as isize == target {
                found = Some(hwnd);
                false
            } else {
                true
            }
        });
        found
    }

    /// One pass of the per-frame guard: the styles winit put back come off again, and a parked
    /// maximize stays clear of the taskbar.
    fn reapply(hwnd: &HWND) {
        strip(hwnd);
        keep_clear_of_the_taskbar(hwnd);
    }

    /// Removes `WS_CAPTION`/`WS_BORDER` and has Windows recalculate the frame. A no-op once they
    /// are gone, so it is cheap to call every frame.
    fn strip(hwnd: &HWND) {
        let style = hwnd.style();
        let without = borderless(style);
        if without.raw() == style.raw() {
            return;
        }
        hwnd.set_style(without);
        let _ = hwnd.SetWindowPos(
            HwndPlace::None,
            POINT::new(),
            SIZE::new(),
            co::SWP::FRAMECHANGED | co::SWP::NOMOVE | co::SWP::NOSIZE | co::SWP::NOZORDER | co::SWP::NOACTIVATE,
        );
    }

    /// How far a parked maximize reaches past its monitor on the left, top and right: the resize
    /// border (about 8 pt at 100 %, more with DPI). Past that, a maximized window's rect is not
    /// parked but being dragged or snapped, and is left alone.
    const FRAME: i32 = 32;

    /// A rect in desktop coordinates: `(left, top, right, bottom)`, right and bottom exclusive.
    type Rect = (i32, i32, i32, i32);

    /// A monitor edge a taskbar can sit on.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Edge {
        Left,
        Top,
        Right,
        Bottom,
    }

    /// Keeps a maximized window clear of the taskbar: one whose rect runs under a taskbar that
    /// reserves its space is trimmed back to the work area plus its frame (#3144), and one covering
    /// its whole monitor stops a pixel short of each auto-hiding taskbar's edge. No-op otherwise.
    fn keep_clear_of_the_taskbar(hwnd: &HWND) {
        if !hwnd.IsZoomed() {
            return;
        }
        let (Ok(r), Ok(mi)) = (hwnd.GetWindowRect(), hwnd.MonitorFromWindow(co::MONITOR::DEFAULTTONEAREST).GetMonitorInfo()) else {
            return;
        };
        let (m, w) = (mi.rcMonitor, mi.rcWork);
        let rect = (r.left, r.top, r.right, r.bottom);
        let monitor = (m.left, m.top, m.right, m.bottom);
        let work = (w.left, w.top, w.right, w.bottom);
        let target = match clamped_height(rect, monitor, work) {
            Some(height) => Some((rect.0, rect.1, rect.2, rect.1.saturating_add(height))),
            // The taskbars are only looked up while the window covers its whole monitor, which a
            // trimmed maximize no longer does.
            None if covers(rect, monitor) => short_of_autohide_taskbars(rect, monitor, work, &taskbar_edges(monitor)),
            None => None,
        };
        let Some((left, top, right, bottom)) = target else {
            return;
        };
        let _ = hwnd.SetWindowPos(
            HwndPlace::None,
            POINT { x: left, y: top },
            SIZE { cx: right.saturating_sub(left), cy: bottom.saturating_sub(top) },
            co::SWP::NOZORDER | co::SWP::NOACTIVATE,
        );
    }

    /// How long a look for the taskbars holds before the windows are listed again.
    const TASKBAR_RECHECK: std::time::Duration = std::time::Duration::from_secs(2);

    /// The edges of `monitor` with a taskbar on them (the primary `Shell_TrayWnd` or a secondary
    /// monitor's `Shell_SecondaryTrayWnd`), shown or hidden. Cached per monitor for
    /// [`TASKBAR_RECHECK`]: it lists every top-level window.
    fn taskbar_edges(monitor: Rect) -> Vec<Edge> {
        type Seen = Option<(std::time::Instant, Rect, Vec<Edge>)>;
        static SEEN: std::sync::Mutex<Seen> = std::sync::Mutex::new(None);
        let mut seen = SEEN.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((at, m, edges)) = seen.as_ref()
            && *m == monitor
            && at.elapsed() < TASKBAR_RECHECK
        {
            return edges.clone();
        }
        let mut edges = Vec::new();
        let _ = winsafe::EnumWindows(|hwnd| {
            let taskbar = hwnd.GetClassName().is_ok_and(|c| c == "Shell_TrayWnd" || c == "Shell_SecondaryTrayWnd");
            if taskbar && let Ok(r) = hwnd.GetWindowRect() {
                edges.extend(taskbar_edge((r.left, r.top, r.right, r.bottom), monitor));
            }
            true
        });
        *seen = Some((std::time::Instant::now(), monitor, edges.clone()));
        edges
    }

    /// The edge of `monitor` taskbar window `bar` sits on: the one nearest its centre, or `None`
    /// when it is on another monitor. A hidden taskbar is slid (mostly) past its edge with a sliver
    /// left on the monitor, so its centre may lie outside it.
    fn taskbar_edge(bar: Rect, monitor: Rect) -> Option<Edge> {
        let (bl, bt, br, bb) = (i64::from(bar.0), i64::from(bar.1), i64::from(bar.2), i64::from(bar.3));
        let (ml, mt, mr, mb) = (i64::from(monitor.0), i64::from(monitor.1), i64::from(monitor.2), i64::from(monitor.3));
        if bl >= br || bt >= bb || ml >= mr || mt >= mb || br <= ml || bl >= mr || bb <= mt || bt >= mb {
            return None;
        }
        // Doubled centre and distances, so nothing rounds.
        let (cx, cy) = (bl + br, bt + bb);
        [(Edge::Left, cx - 2 * ml), (Edge::Top, cy - 2 * mt), (Edge::Right, 2 * mr - cx), (Edge::Bottom, 2 * mb - cy)]
            .into_iter()
            .min_by_key(|&(_, distance)| distance)
            .map(|(edge, _)| edge)
    }

    /// Whether `rect` reaches every edge of `monitor`: what the shell takes for a full-screen app
    /// when the window has no caption.
    fn covers(rect: Rect, monitor: Rect) -> bool {
        rect.0 <= monitor.0 && rect.1 <= monitor.1 && rect.2 >= monitor.2 && rect.3 >= monitor.3
    }

    /// A maximize covering its monitor, trimmed to stop one pixel short of each edge where a
    /// taskbar reserves no work area (it auto-hides), so the shell still reveals it there. `None`
    /// when there is nothing to trim.
    fn short_of_autohide_taskbars(rect: Rect, monitor: Rect, work: Rect, taskbars: &[Edge]) -> Option<Rect> {
        if !covers(rect, monitor) {
            return None;
        }
        let (mut l, mut t, mut r, mut b) = rect;
        for edge in taskbars {
            match edge {
                Edge::Left if work.0 == monitor.0 => l = monitor.0.checked_add(1)?,
                Edge::Top if work.1 == monitor.1 => t = monitor.1.checked_add(1)?,
                Edge::Right if work.2 == monitor.2 => r = monitor.2.checked_sub(1)?,
                Edge::Bottom if work.3 == monitor.3 => b = monitor.3.checked_sub(1)?,
                _ => {}
            }
        }
        let trimmed = (l, t, r, b);
        (l < r && t < b && trimmed != rect).then_some(trimmed)
    }

    /// The height to trim a maximize to so its rect stops at the taskbar, or `None` when it
    /// already does or `rect` (desktop coordinates) is not parked at its maximize place. `monitor`
    /// is the monitor's full rect and `work` its work area (the monitor less the taskbar) in the
    /// same coordinates. The parked check is against the full monitor, because that is where
    /// Windows puts a maximize; the trim is against the work area.
    fn clamped_height(rect: Rect, monitor: Rect, work: Rect) -> Option<i32> {
        let (l, t, r, b) = rect;
        let (ml, mt, mr, _) = monitor;
        let (_, _, _, wb) = work;
        let side = ml - l;
        if !(0..=FRAME).contains(&side) || !(0..=FRAME).contains(&(mt - t)) || !(0..=FRAME).contains(&(r - mr)) {
            return None;
        }
        if b <= wb + side {
            return None;
        }
        Some(wb + side - t)
    }

    /// `style` without the OS title bar and frame (`WS_CAPTION`, which already includes
    /// `WS_BORDER`). The sizing border (`WS_THICKFRAME`) stays, so edge resizing and Aero snap
    /// keep working, as they did with winit 0.27's undecorated windows.
    fn borderless(style: co::WS) -> co::WS {
        style & !(co::WS::CAPTION | co::WS::BORDER)
    }

    #[cfg(test)]
    mod tests {
        use super::{Edge, borderless, clamped_height, covers, short_of_autohide_taskbars, taskbar_edge};
        use winsafe::co;

        #[test]
        fn the_borderless_style_drops_the_caption_and_border_only() {
            // `WS_CAPTION` is 0x00c0_0000 and already contains `WS_BORDER` (0x0080_0000).
            let decorated = co::WS::CAPTION | co::WS::BORDER | co::WS::SYSMENU | co::WS::THICKFRAME | co::WS::MAXIMIZEBOX | co::WS::MINIMIZEBOX;
            let bare = borderless(decorated);
            assert!(!bare.has(co::WS::CAPTION), "caption gone");
            assert!(!bare.has(co::WS::BORDER), "border gone");
            assert!(bare.has(co::WS::THICKFRAME), "edge resizing kept");
            assert!(bare.has(co::WS::MAXIMIZEBOX) && bare.has(co::WS::SYSMENU), "window buttons kept");
            // Already borderless: unchanged, so the per-frame call does nothing.
            assert_eq!(bare.raw(), borderless(bare).raw());
        }

        #[test]
        fn a_maximize_covering_the_taskbar_is_trimmed_to_the_work_area() {
            // 1920 × 1080 with a 34 pt taskbar: a caption-less maximize covers the whole
            // monitor, taskbar included (#3144). Trimmed so the rect ends at the work area
            // plus the frame the window was parked with.
            let monitor = (0, 0, 1920, 1080);
            let work = (0, 0, 1920, 1046);
            assert_eq!(clamped_height((-7, -7, 1927, 1087), monitor, work), Some(1046 + 7 - (-7)));
        }

        #[test]
        fn a_maximize_already_clear_of_the_taskbar_is_left_alone() {
            // What Windows sizes a framed maximize to: the work area plus the frame.
            assert_eq!(clamped_height((-8, -8, 1928, 1054), (0, 0, 1920, 1080), (0, 0, 1920, 1046)), None);
        }

        #[test]
        fn a_maximize_beside_a_side_taskbar_is_left_alone() {
            // Taskbar on the left: the work area starts further right, but the monitor's bottom
            // is still clear, so the rect reaches nothing.
            assert_eq!(clamped_height((-8, -8, 1928, 1088), (0, 0, 1920, 1080), (48, 0, 1920, 1080)), None);
        }

        /// An auto-hiding taskbar leaves the work area whole, so the maximize covers the monitor
        /// (frame included) and the shell takes it for full screen. It is trimmed to stop a pixel
        /// short of that edge instead; measured on a 2560 × 1440 monitor, where the old trim to
        /// `(-8, -8, 2568, 1447)` still kept the taskbar down.
        #[test]
        fn a_maximize_leaves_a_pixel_strip_on_an_auto_hidden_taskbar_edge() {
            let monitor = (0, 0, 2560, 1440);
            let rect = (-8, -8, 2568, 1448);
            assert_eq!(clamped_height(rect, monitor, monitor), None, "no reserved area to clamp to");
            for (edge, expected) in [
                (Edge::Bottom, (-8, -8, 2568, 1439)),
                (Edge::Top, (-8, 1, 2568, 1448)),
                (Edge::Left, (1, -8, 2568, 1448)),
                (Edge::Right, (-8, -8, 2559, 1448)),
            ] {
                assert_eq!(short_of_autohide_taskbars(rect, monitor, monitor, &[edge]), Some(expected), "{edge:?}");
                assert!(!covers(expected, monitor), "{edge:?} no longer reads as full screen");
                assert_eq!(short_of_autohide_taskbars(expected, monitor, monitor, &[edge]), None, "{edge:?} already trimmed");
            }
            assert!(covers((-8, -8, 2568, 1447), monitor), "the old trim still covered the monitor");
            assert_eq!(short_of_autohide_taskbars(rect, monitor, monitor, &[]), None, "no taskbar");
            // A taskbar that reserves its work area is the clamp's job.
            assert_eq!(short_of_autohide_taskbars(rect, monitor, (0, 0, 2560, 1392), &[Edge::Bottom]), None);
            assert_eq!(short_of_autohide_taskbars((0, 0, 1280, 1440), monitor, monitor, &[Edge::Bottom]), None, "half-screen");
        }

        /// Taskbar rects as a three-monitor desktop reports them, shown and hidden.
        #[test]
        fn a_taskbar_sits_on_the_monitor_edge_nearest_its_centre() {
            let monitor = (0, 0, 2560, 1440);
            assert_eq!(taskbar_edge((0, 1392, 2560, 1440), monitor), Some(Edge::Bottom), "shown");
            assert_eq!(taskbar_edge((0, 1438, 2560, 1486), monitor), Some(Edge::Bottom), "auto-hidden");
            assert_eq!(taskbar_edge((0, -46, 2560, 2), monitor), Some(Edge::Top));
            assert_eq!(taskbar_edge((-60, 0, 2, 1440), monitor), Some(Edge::Left));
            assert_eq!(taskbar_edge((2558, 0, 2620, 1440), monitor), Some(Edge::Right));
            assert_eq!(taskbar_edge((2560, 1438, 5120, 1486), monitor), None, "the right-hand monitor's");
            assert_eq!(taskbar_edge((-2560, 1598, 0, 1658), monitor), None, "the left-hand monitor's");
            assert_eq!(taskbar_edge((-2560, 1598, 0, 1658), (-2560, 0, 0, 1600)), Some(Edge::Bottom));
            assert!(taskbar_edge((i32::MIN, i32::MIN, i32::MAX, i32::MAX), monitor).is_some(), "no overflow");
        }

        #[test]
        fn a_maximize_being_dragged_away_is_left_alone() {
            let monitor = (0, 0, 1920, 1080);
            let work = (0, 0, 1920, 1046);
            assert_eq!(clamped_height((100, 200, 1100, 900), monitor, work), None, "moved");
            assert_eq!(clamped_height((-500, -7, 1427, 1087), monitor, work), None, "resized");
        }
    }
}

#[cfg(not(target_os = "windows"))]
mod platform {
    use photocraft_ui_egui::WindowFrameFn;

    pub fn service(_cc: &eframe::CreationContext<'_>, _custom_titlebar: bool) -> Option<WindowFrameFn> {
        None
    }
}
