use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    Frame,
};

use crate::app::AppState;
use crate::layout::PaneInfo;

pub(crate) fn pane_scrollbar_rect(info: &PaneInfo) -> Option<Rect> {
    info.scrollbar_rect
}

pub(crate) fn release_notes_scrollbar_rect(
    body: Rect,
    metrics: crate::pane::ScrollMetrics,
) -> Option<Rect> {
    (should_show_scrollbar(metrics) && body.width > 1).then_some(Rect::new(
        body.x + body.width - 1,
        body.y,
        1,
        body.height,
    ))
}

/// Scroll metrics for a view that draws `view_rows` of a taller grid,
/// starting `rows_above` rows down. Grid rows outside the view count as
/// scrollable rows, so the pane scrollbar marks the view as cropped and shows
/// which slice it draws.
pub(crate) fn cropped_view_scroll_metrics(
    metrics: crate::pane::ScrollMetrics,
    screen_rows: u16,
    view_rows: u16,
    rows_above: u16,
) -> crate::pane::ScrollMetrics {
    let hidden = usize::from(screen_rows.saturating_sub(view_rows));
    if hidden == 0 {
        return metrics;
    }
    let below = hidden.saturating_sub(usize::from(rows_above));
    crate::pane::ScrollMetrics {
        offset_from_bottom: metrics.offset_from_bottom.saturating_add(below),
        max_offset_from_bottom: metrics.max_offset_from_bottom.saturating_add(hidden),
        viewport_rows: usize::from(view_rows),
    }
}

/// The pane's scroll metrics as a view `view_rows` tall presents them.
pub(crate) fn pane_view_scroll_metrics(
    rt: &crate::terminal::TerminalRuntime,
    view_rows: u16,
) -> Option<crate::pane::ScrollMetrics> {
    let metrics = rt.scroll_metrics()?;
    let screen_rows = rt.current_size().0;
    if screen_rows <= view_rows {
        return Some(metrics);
    }
    Some(cropped_view_scroll_metrics(
        metrics,
        screen_rows,
        view_rows,
        rt.crop_rows_above(view_rows),
    ))
}

pub(crate) fn should_show_scrollbar(metrics: crate::pane::ScrollMetrics) -> bool {
    metrics.max_offset_from_bottom > 0
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ScrollbarThumb {
    pub top: u16,
    pub len: u16,
}

pub(crate) fn scrollbar_thumb(
    metrics: crate::pane::ScrollMetrics,
    track: Rect,
) -> Option<ScrollbarThumb> {
    if metrics.max_offset_from_bottom == 0 || track.height == 0 {
        return None;
    }

    let track_height = track.height as usize;
    let total_rows = metrics.max_offset_from_bottom + metrics.viewport_rows;
    if total_rows == 0 {
        return None;
    }

    let thumb_len = ((metrics.viewport_rows * track_height) as f32 / total_rows as f32)
        .round()
        .max(1.0)
        .min(track_height as f32) as usize;
    let max_thumb_top = track_height.saturating_sub(thumb_len);
    let scrolled_from_top = metrics
        .max_offset_from_bottom
        .saturating_sub(metrics.offset_from_bottom);
    let thumb_top = if max_thumb_top == 0 || metrics.max_offset_from_bottom == 0 {
        0
    } else {
        ((scrolled_from_top * max_thumb_top) as f32 / metrics.max_offset_from_bottom as f32)
            .round()
            .clamp(0.0, max_thumb_top as f32) as usize
    };

    Some(ScrollbarThumb {
        top: track.y + thumb_top as u16,
        len: thumb_len as u16,
    })
}

pub(crate) fn scrollbar_thumb_grab_offset(
    metrics: crate::pane::ScrollMetrics,
    track: Rect,
    row: u16,
) -> Option<u16> {
    let thumb = scrollbar_thumb(metrics, track)?;
    (row >= thumb.top && row < thumb.top + thumb.len).then(|| row - thumb.top)
}

fn scrollbar_offset_from_thumb_top(
    metrics: crate::pane::ScrollMetrics,
    track: Rect,
    thumb_top: usize,
) -> usize {
    if metrics.max_offset_from_bottom == 0 {
        return 0;
    }

    let thumb_len = scrollbar_thumb(metrics, track)
        .map(|thumb| thumb.len as usize)
        .unwrap_or(1);
    let max_thumb_top = track.height as usize - thumb_len.min(track.height as usize);
    if max_thumb_top == 0 {
        return 0;
    }

    let desired_top = thumb_top.min(max_thumb_top);
    let scrolled_from_top = ((desired_top * metrics.max_offset_from_bottom) as f32
        / max_thumb_top as f32)
        .round() as usize;
    metrics
        .max_offset_from_bottom
        .saturating_sub(scrolled_from_top)
}

pub(crate) fn scrollbar_offset_from_row(
    metrics: crate::pane::ScrollMetrics,
    track: Rect,
    row: u16,
) -> usize {
    let thumb = match scrollbar_thumb(metrics, track) {
        Some(thumb) => thumb,
        None => return 0,
    };
    let clamped_row = row.clamp(track.y, track.y + track.height.saturating_sub(1));
    let row_offset = clamped_row.saturating_sub(track.y) as usize;
    let thumb_center = (thumb.len as usize) / 2;
    let desired_top = row_offset.saturating_sub(thumb_center);
    scrollbar_offset_from_thumb_top(metrics, track, desired_top)
}

pub(crate) fn scrollbar_offset_from_drag_row(
    metrics: crate::pane::ScrollMetrics,
    track: Rect,
    row: u16,
    grab_row_offset: u16,
) -> usize {
    let clamped_row = row.clamp(track.y, track.y + track.height.saturating_sub(1));
    let row_offset = clamped_row.saturating_sub(track.y) as usize;
    let desired_top = row_offset.saturating_sub(grab_row_offset as usize);
    scrollbar_offset_from_thumb_top(metrics, track, desired_top)
}

pub(crate) fn render_scrollbar_buffer(
    buffer: &mut Buffer,
    metrics: crate::pane::ScrollMetrics,
    track: Rect,
    track_color: Color,
    thumb_color: Color,
    thumb_symbol: &str,
) {
    if metrics.max_offset_from_bottom == 0 {
        return;
    }

    let Some(thumb) = scrollbar_thumb(metrics, track) else {
        return;
    };

    for y in track.y..track.y + track.height {
        let cell = &mut buffer[(track.x, y)];
        cell.set_symbol("▕");
        cell.set_style(Style::default().fg(track_color));
    }
    for y in thumb.top..thumb.top + thumb.len {
        let cell = &mut buffer[(track.x, y)];
        cell.set_symbol(thumb_symbol);
        cell.set_style(Style::default().fg(thumb_color));
    }
}

pub(crate) fn render_pane_scrollbar_buffer(
    buffer: &mut Buffer,
    metrics: crate::pane::ScrollMetrics,
    track: Rect,
    palette: &crate::app::state::Palette,
    focused: bool,
) {
    let (track_color, thumb_color, thumb_symbol) = if focused {
        (palette.overlay0, palette.overlay1, "▐")
    } else {
        (palette.surface_dim, palette.overlay0, "▕")
    };
    render_scrollbar_buffer(
        buffer,
        metrics,
        track,
        track_color,
        thumb_color,
        thumb_symbol,
    );
}

pub(super) fn render_pane_scrollbar(
    app: &AppState,
    frame: &mut Frame,
    info: &PaneInfo,
    rt: &crate::terminal::TerminalRuntime,
) {
    let Some(track) = pane_scrollbar_rect(info) else {
        return;
    };
    let Some(metrics) = pane_view_scroll_metrics(rt, info.inner_rect.height) else {
        return;
    };
    render_pane_scrollbar_buffer(
        frame.buffer_mut(),
        metrics,
        track,
        &app.palette,
        info.is_focused,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pane::ScrollMetrics;

    #[test]
    fn cropped_view_scrollbar_counts_the_rows_the_view_hides() {
        let live = ScrollMetrics {
            offset_from_bottom: 0,
            max_offset_from_bottom: 0,
            viewport_rows: 109,
        };
        // A view as tall as the grid keeps the grid's metrics.
        assert_eq!(cropped_view_scroll_metrics(live, 109, 109, 0), live);
        // A 60-row view of a 109-row grid with no history shows a scrollbar.
        let cropped = cropped_view_scroll_metrics(live, 109, 60, 49);
        assert_eq!(
            cropped,
            ScrollMetrics {
                offset_from_bottom: 0,
                max_offset_from_bottom: 49,
                viewport_rows: 60,
            }
        );
        assert!(should_show_scrollbar(cropped));
        let track = Rect::new(0, 0, 1, 60);
        let thumb = scrollbar_thumb(cropped, track).unwrap();
        assert_eq!(thumb.top + thumb.len, 60, "a live view sits at the bottom");
        assert!(thumb.len < 60);

        // The same view drawing the top of the grid sits at the top.
        let top = cropped_view_scroll_metrics(live, 109, 60, 0);
        assert_eq!(top.offset_from_bottom, 49);
        assert_eq!(scrollbar_thumb(top, track).unwrap().top, 0);
    }

    #[tokio::test]
    async fn pane_view_scroll_metrics_crop_a_runtime_taller_than_the_view() {
        let lines = (0..10)
            .map(|i| format!("L{i}"))
            .collect::<Vec<_>>()
            .join("\r\n");
        let runtime =
            crate::terminal::TerminalRuntime::test_with_screen_bytes(20, 10, lines.as_bytes());
        assert_eq!(
            pane_view_scroll_metrics(&runtime, 10),
            runtime.scroll_metrics()
        );
        let metrics = pane_view_scroll_metrics(&runtime, 4).unwrap();
        assert_eq!(metrics.viewport_rows, 4);
        assert_eq!(metrics.offset_from_bottom, 0);
        assert_eq!(
            metrics.max_offset_from_bottom,
            runtime.scroll_metrics().unwrap().max_offset_from_bottom + 6
        );
        assert_eq!(runtime.crop_rows_above(4), 6);
        let cursor = runtime.cursor_state(Rect::new(5, 2, 20, 4), true).unwrap();
        assert_eq!((cursor.x, cursor.y), (5 + 2, 2 + 3));
    }
}
