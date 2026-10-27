use super::*;

fn rect_fits_frame(rect: protocol::SurfaceRect, frame: &FrameData) -> bool {
    rect.x.saturating_add(rect.width) <= frame.width
        && rect.y.saturating_add(rect.height) <= frame.height
}

/// Rebinds the patch's OSC 8 cells to `committed`, the recipient's frame
/// table, so patched cells and the recipient's retained cells share one
/// table. None when the recipient has not received one of the URIs; only a
/// complete frame (or surface delta) can add it.
fn bind_patch_hyperlinks<'a>(
    patch: &'a crate::pane::TerminalDirtyPatch,
    committed: &[String],
) -> Option<std::borrow::Cow<'a, crate::pane::TerminalDirtyPatch>> {
    if patch.hyperlinks.is_empty() {
        return Some(std::borrow::Cow::Borrowed(patch));
    }
    let bindings = patch
        .hyperlinks
        .iter()
        .map(|uri| {
            let index = committed.iter().position(|known| known == uri)?;
            u32::try_from(index).ok()
        })
        .collect::<Option<Vec<_>>>()?;
    let mut bound = patch.clone();
    for cell in bound
        .rows
        .iter_mut()
        .flat_map(|(_, cells)| cells.iter_mut())
    {
        if let Some(index) = cell.hyperlink {
            cell.hyperlink = Some(*bindings.get(usize::try_from(index).ok()?)?);
        }
    }
    bound.hyperlinks = committed.to_vec();
    Some(std::borrow::Cow::Owned(bound))
}

fn patch_row_changed(frame: &FrameData, row: &protocol::PaneSurfacePatchRow) -> Option<bool> {
    if row.y >= frame.height
        || row.x.saturating_add(u16::try_from(row.cells.len()).ok()?) > frame.width
    {
        return None;
    }
    let start = usize::from(row.y) * usize::from(frame.width) + usize::from(row.x);
    let end = start + row.cells.len();
    if end > frame.cells.len() {
        return None;
    }
    Some(frame.cells[start..end] != row.cells)
}

fn changed_rows(
    frame: &FrameData,
    area: protocol::SurfaceRect,
    patch: &crate::pane::TerminalDirtyPatch,
) -> Option<Vec<protocol::PaneSurfacePatchRow>> {
    if !rect_fits_frame(area, frame) {
        return None;
    }
    let mut rows = Vec::new();
    for (local_y, cells) in &patch.rows {
        if *local_y >= area.height {
            continue;
        }
        let width = usize::from(area.width);
        if cells.len() < width {
            return None;
        }
        let y = area.y + *local_y;
        let frame_start = usize::from(y) * usize::from(frame.width) + usize::from(area.x);
        let frame_end = frame_start.checked_add(width)?;
        let existing = frame.cells.get(frame_start..frame_end)?;
        let desired = &cells[..width];
        let mut offset = 0;
        while offset < width {
            if existing[offset] == desired[offset] {
                offset += 1;
                continue;
            }
            let start = offset;
            offset += 1;
            while offset < width && existing[offset] != desired[offset] {
                offset += 1;
            }
            // Include the following cell so a wide-to-narrow (or
            // narrow-to-wide) transition repaints content covered by the old
            // grapheme width even when that logical neighbor is unchanged.
            let end = offset.saturating_add(1).min(width);
            rows.push(protocol::PaneSurfacePatchRow {
                x: area.x.checked_add(u16::try_from(start).ok()?)?,
                y,
                cells: desired[start..end].to_vec(),
            });
            offset = end;
        }
    }
    Some(rows)
}

// A clipped wide lead cannot be updated independently of the dense renderer:
// ratatui's flat-buffer diff may skip the first cell of the following row.
// Preserve that existing dense behavior here; this does not repair its clipping.
// Inspect one edge cell per row, never the complete recipient grid.
fn terminal_has_clipped_wide_edge(
    frame: &FrameData,
    patch: Option<&crate::pane::TerminalDirtyPatch>,
) -> bool {
    let width = usize::from(frame.width);
    if width == 0 {
        return false;
    }
    let edge_is_wide =
        |cell: &protocol::CellData| crate::protocol::render_ansi::cell_width(cell) > 1;
    if let Some(patch) = patch {
        patch
            .rows
            .iter()
            .any(|(y, cells)| *y < frame.height && cells.get(width - 1).is_some_and(edge_is_wide))
    } else {
        frame
            .cells
            .chunks_exact(width)
            .any(|row| edge_is_wide(&row[width - 1]))
    }
}

fn retained_scrollbar_patch(
    app: &app::App,
    frame: &FrameData,
    pane: &mut protocol::PaneSurfacePane,
    alternate_screen_active: bool,
    metrics: Option<crate::pane::ScrollMetrics>,
) -> Option<Vec<protocol::PaneSurfacePatchRow>> {
    let next_rect = metrics
        .filter(|metrics| metrics.max_offset_from_bottom > 0)
        .filter(|_| app.state.pane_scrollbars && !alternate_screen_active)
        .and_then(|_| {
            let rect = protocol::SurfaceRect {
                x: pane.inner_rect.x.checked_add(pane.inner_rect.width)?,
                y: pane.inner_rect.y,
                width: 1,
                height: pane.inner_rect.height,
            };
            (rect_fits_frame(rect, frame)
                && rect.x >= pane.rect.x
                && rect.x < pane.rect.x.saturating_add(pane.rect.width))
            .then_some(rect)
        });
    let patch_rect = next_rect.or(pane.scrollbar_rect);
    pane.scrollbar_rect = next_rect;
    let Some(rect) = patch_rect else {
        return Some(Vec::new());
    };

    let track = Rect::new(0, 0, 1, rect.height);
    let mut buffer = ratatui::buffer::Buffer::empty(track);
    if let (Some(metrics), Some(_)) = (metrics, next_rect) {
        crate::ui::render_pane_scrollbar_buffer(
            &mut buffer,
            metrics,
            track,
            &app.state.palette,
            pane.focused,
        );
    }
    let cells = buffer
        .content
        .iter()
        .map(protocol::CellData::from_ratatui_cell)
        .collect::<Vec<_>>();
    let mut rows = Vec::new();
    for (offset, cell) in cells.into_iter().enumerate() {
        let row = protocol::PaneSurfacePatchRow {
            x: rect.x,
            y: rect.y.checked_add(u16::try_from(offset).ok()?)?,
            cells: vec![cell],
        };
        if patch_row_changed(frame, &row)? {
            rows.push(row);
        }
    }
    Some(rows)
}

fn retained_cursor(
    app: &app::App,
    panes: &[protocol::PaneSurfacePane],
) -> Option<protocol::CursorState> {
    let pane = panes.iter().find(|pane| pane.focused)?;
    let (workspace_index, pane_id) = app.parse_pane_id(&pane.pane_id)?;
    if !app.state.pane_exposes_host_cursor(workspace_index, pane_id) {
        return None;
    }
    let runtime = app.state.runtime_for_pane_in_workspace(
        &app.terminal_runtimes,
        workspace_index,
        pane_id,
    )?;
    if runtime.synchronized_output_active() {
        return None;
    }
    let area = Rect::new(
        pane.inner_rect.x,
        pane.inner_rect.y,
        pane.inner_rect.width,
        pane.inner_rect.height,
    );
    runtime
        .cursor_state(area, true)
        .map(|cursor| protocol::CursorState {
            x: cursor.x,
            y: cursor.y,
            visible: cursor.visible && !crate::ui::pane_is_scrolled_back(runtime),
            shape: cursor.shape,
        })
}

struct RetainedRecipient<'a> {
    client_id: u64,
    surface: &'a protocol::PaneSurfaceFrame,
}

struct RetainedTerminalRecipient<'a> {
    client_id: u64,
    terminal_id: &'a str,
    frame: &'a FrameData,
}

struct CollectedPanePatch {
    pane_id: String,
    terminal_id: Option<String>,
    synchronized_epoch: u64,
    patch: crate::pane::TerminalDirtyPatch,
    content_revision: u64,
    scroll_metrics: Option<crate::pane::ScrollMetrics>,
    screen_rows: u16,
    mouse_reporting: bool,
    sgr_pixel_mouse: bool,
    alternate_screen_active: bool,
    graphics_may_have_placements: bool,
}

struct RetainedRecipientUpdate {
    client_id: u64,
    patch: protocol::PaneSurfacePatch,
    graphics: Option<(
        protocol::PaneSurfaceFrame,
        crate::kitty_graphics::surface::DeliveryCache,
        crate::kitty_graphics::surface::SourceFiles,
    )>,
}

pub(super) fn has_synchronized_pane(app: &app::App, surface: &protocol::PaneSurfaceFrame) -> bool {
    surface.panes.iter().any(|pane| {
        app.parse_pane_id(&pane.pane_id)
            .and_then(|(workspace_index, pane_id)| {
                app.state.runtime_for_pane_in_workspace(
                    &app.terminal_runtimes,
                    workspace_index,
                    pane_id,
                )
            })
            .is_some_and(|runtime| runtime.synchronized_output_active())
    })
}

impl HeadlessServer {
    /// Applies terminal dirty rows to the committed origin-relative pane surface.
    /// Any presentation or geometry uncertainty falls back to the complete renderer.
    pub(super) fn render_retained_pane_surface_and_stream(
        &mut self,
        pty_sources: &HashSet<crate::layout::PaneId>,
    ) -> bool {
        crate::render_prof::event("retained_surface.attempt");
        let started = crate::render_prof::timer();
        macro_rules! fallback {
            ($reason:literal) => {{
                crate::render_prof::event(concat!("retained_surface.fallback.", $reason));
                crate::render_prof::duration_since("retained_surface.total", started);
                return false;
            }};
        }
        macro_rules! success {
            ($reason:literal) => {{
                crate::render_prof::event("retained_surface.success");
                crate::render_prof::event(concat!("retained_surface.success.", $reason));
                crate::render_prof::duration_since("retained_surface.total", started);
                return true;
            }};
        }

        if pty_sources.is_empty()
            || self.app.full_redraw_pending
            || self.app.state.popup_pane.is_some()
            || self.app.state.reveal_hidden_cursor_for_cjk_ime
        {
            fallback!("unsafe_state");
        }
        let mut targets = render_targets(&self.clients, self.foreground_client_id);
        targets.retain(|(client_id, _, _, _, mode)| {
            !matches!(mode, ClientConnectionMode::ClientShell)
                || self
                    .clients
                    .get(client_id)
                    .is_some_and(|client| client.shell_surface_active)
        });
        if targets.is_empty() {
            success!("no_active_surface");
        }
        let mut recipients = Vec::with_capacity(targets.len());
        let mut terminal_recipients = Vec::new();
        for (client_id, (cols, rows), _, _, mode) in &targets {
            let Some(client) = self.clients.get(client_id) else {
                fallback!("client_missing");
            };
            if client.deferred_render() != DeferredRender::None {
                crate::render_prof::event("retained_surface.recipient_deferred");
                continue;
            }
            if let ClientConnectionMode::TerminalAttach { terminal_id }
            | ClientConnectionMode::TerminalObserve { terminal_id } = mode
            {
                let Some(frame) = client.render_state.last_terminal_frame() else {
                    fallback!("terminal_no_baseline");
                };
                if frame.width != *cols
                    || frame.height != *rows
                    || !frame.hyperlinks.is_empty()
                    || !frame.graphics.is_empty()
                {
                    fallback!("terminal_baseline_mismatch");
                }
                if terminal_has_clipped_wide_edge(frame, None) {
                    fallback!("terminal_clipped_wide_baseline");
                }
                let Some(runtime) = self.runtime_for_terminal_id_string(terminal_id) else {
                    fallback!("runtime_missing");
                };
                if runtime.synchronized_output_active() {
                    fallback!("synchronized_visible");
                }
                terminal_recipients.push(RetainedTerminalRecipient {
                    client_id: *client_id,
                    terminal_id,
                    frame,
                });
                continue;
            }
            if client.render_state.requires_recompute() {
                fallback!("recompute_pending");
            }
            let Some(surface) = client.render_state.last_pane_surface() else {
                fallback!("no_baseline");
            };
            if surface.boot_id != self.client_shell_boot_id
                || surface.projection_revision != client.shell_projection_revision
                || surface.frame.width != *cols
                || surface.frame.height != *rows
                || surface.popup.is_some()
                || !surface.graphics.assets.is_empty()
                || !surface.frame.graphics.is_empty()
            {
                fallback!("baseline_mismatch");
            }
            if has_synchronized_pane(&self.app, surface) {
                fallback!("synchronized_visible");
            }
            recipients.push(RetainedRecipient {
                client_id: *client_id,
                surface,
            });
        }
        if recipients.is_empty() && terminal_recipients.is_empty() {
            success!("all_recipients_deferred");
        }

        let mut collected = Vec::with_capacity(pty_sources.len());
        for source in pty_sources {
            let mut public_pane_id = None;
            let terminal_id = self.terminal_id_for_pane(*source).map(ToString::to_string);
            let mut width = 0u16;
            let mut height = 0u16;
            let mut min_height = u16::MAX;
            for recipient in &recipients {
                let Some(pane) = recipient.surface.panes.iter().find(|pane| {
                    self.app
                        .parse_pane_id(&pane.pane_id)
                        .is_some_and(|(_, pane_id)| pane_id == *source)
                }) else {
                    continue;
                };
                public_pane_id.get_or_insert_with(|| pane.pane_id.clone());
                width = width.max(pane.inner_rect.width);
                height = height.max(pane.inner_rect.height);
                min_height = min_height.min(pane.inner_rect.height);
            }
            for recipient in &terminal_recipients {
                if terminal_id.as_deref() != Some(recipient.terminal_id) {
                    continue;
                }
                if public_pane_id.is_none() {
                    public_pane_id =
                        self.app
                            .find_pane(*source)
                            .and_then(|(workspace_index, _)| {
                                self.app.public_pane_id(workspace_index, *source)
                            });
                }
                width = width.max(recipient.frame.width);
                height = height.max(recipient.frame.height);
                min_height = min_height.min(recipient.frame.height);
            }
            let Some(public_pane_id) = public_pane_id else {
                continue;
            };
            let Some((workspace_index, pane_id)) = self.app.parse_pane_id(&public_pane_id) else {
                fallback!("pane_missing");
            };
            let Some(runtime) = self.app.state.runtime_for_pane_in_workspace(
                &self.app.terminal_runtimes,
                workspace_index,
                pane_id,
            ) else {
                fallback!("runtime_missing");
            };
            // Row patches index the grid from its top row; a view shorter
            // than the grid that draws a lower slice takes a full render.
            if runtime.crop_rows_above(min_height) > 0 {
                fallback!("cropped_view");
            }
            let (synchronized, synchronized_epoch) = runtime.synchronized_output_state();
            if synchronized {
                fallback!("synchronized_visible");
            }
            let Some(snapshot) = runtime.collect_dirty_patch_snapshot(width, height) else {
                fallback!("terminal_snapshot");
            };
            let patch = match snapshot.patch {
                crate::pane::TerminalDirtyPatchOutcome::Clean => {
                    crate::render_prof::event("retained_surface.pane_clean");
                    crate::pane::TerminalDirtyPatch {
                        rows: Vec::new(),
                        hyperlinks: Vec::new(),
                    }
                }
                crate::pane::TerminalDirtyPatchOutcome::Patch(patch) => patch,
                crate::pane::TerminalDirtyPatchOutcome::Fallback => {
                    fallback!("terminal_patch");
                }
            };
            collected.push(CollectedPanePatch {
                pane_id: public_pane_id,
                terminal_id,
                synchronized_epoch,
                patch,
                content_revision: snapshot.content_revision,
                scroll_metrics: snapshot.scroll_metrics,
                screen_rows: runtime.current_size().0,
                mouse_reporting: snapshot.mouse_reporting,
                sgr_pixel_mouse: snapshot.sgr_pixel_mouse,
                alternate_screen_active: snapshot.alternate_screen_active,
                graphics_may_have_placements: snapshot.graphics_may_have_placements,
            });
        }

        let mut updates = Vec::with_capacity(recipients.len());
        for recipient in &recipients {
            let client_id = recipient.client_id;
            let surface = recipient.surface;
            let mut panes = surface.panes.clone();
            let projection_revision = surface.projection_revision;
            let base_surface_revision = surface.surface_revision;
            let mut changed_panes = Vec::with_capacity(collected.len());
            let mut patch_rows = Vec::new();
            let mut metadata_changed = false;
            let mut refresh_graphics = !surface.graphics.placements.is_empty()
                || !surface.graphics.retained_assets.is_empty();
            for collected_pane in &collected {
                let Some(pane) = panes
                    .iter_mut()
                    .find(|pane| pane.pane_id == collected_pane.pane_id)
                else {
                    continue;
                };
                // Alternate-screen transitions change whether the pane reserves
                // a scrollbar gutter. Recompute layout and resize the runtime
                // through the complete renderer before retaining further rows.
                if pane.alternate_screen_active != collected_pane.alternate_screen_active {
                    fallback!("alternate_screen_geometry");
                }
                let Some(patch) =
                    bind_patch_hyperlinks(&collected_pane.patch, &surface.frame.hyperlinks)
                else {
                    fallback!("hyperlink_unbound");
                };
                refresh_graphics |= collected_pane.graphics_may_have_placements;
                let previous_pane = pane.clone();
                let Some(rows) = changed_rows(&surface.frame, pane.inner_rect, &patch) else {
                    fallback!("invalid_patch");
                };
                patch_rows.extend(rows);
                // Every retained view draws the grid from its top row (the
                // cropped-view fallback above), so the hidden rows lie below.
                let view_metrics = collected_pane.scroll_metrics.map(|metrics| {
                    crate::ui::cropped_view_scroll_metrics(
                        metrics,
                        collected_pane.screen_rows,
                        pane.inner_rect.height,
                        0,
                    )
                });
                let Some(scrollbar_rows) = retained_scrollbar_patch(
                    &self.app,
                    &surface.frame,
                    pane,
                    collected_pane.alternate_screen_active,
                    view_metrics,
                ) else {
                    fallback!("scrollbar_patch");
                };
                patch_rows.extend(scrollbar_rows);
                pane.content_revision = collected_pane.content_revision;
                pane.mouse_reporting = collected_pane.mouse_reporting;
                pane.sgr_pixel_mouse = collected_pane.sgr_pixel_mouse;
                pane.alternate_screen_active = collected_pane.alternate_screen_active;
                pane.scroll = collected_pane.scroll_metrics.map(|metrics| {
                    protocol::PaneSurfaceScrollMetrics {
                        offset_from_bottom: metrics.offset_from_bottom as u64,
                        max_offset_from_bottom: metrics.max_offset_from_bottom as u64,
                        viewport_rows: metrics.viewport_rows as u64,
                    }
                });
                metadata_changed |= *pane != previous_pane;
                changed_panes.push(pane.clone());
            }

            let cursor = retained_cursor(&self.app, &panes);
            let cursor_changed = cursor != surface.frame.cursor;
            let patch = protocol::PaneSurfacePatch {
                boot_id: self.client_shell_boot_id.clone(),
                projection_revision,
                base_surface_revision,
                surface_revision: 0,
                rows: patch_rows,
                panes: changed_panes,
                cursor,
            };
            let mut graphics_changed = false;
            let graphics = if refresh_graphics {
                let Some(target) = self.shell_target_for_client(client_id) else {
                    fallback!("graphics_target");
                };
                let client = &self.clients[&client_id];
                let Some((graphics, delivery, sources)) =
                    crate::server::client_shell_graphics::collect_retained(
                        &self.app,
                        &panes,
                        target,
                        client.cell_size,
                        &client.shell_graphics_delivery,
                        client_id,
                    )
                else {
                    fallback!("graphics_geometry");
                };
                graphics_changed = graphics != surface.graphics;
                Some((graphics, delivery, sources))
            } else {
                None
            };
            if patch.rows.is_empty() && !cursor_changed && !metadata_changed && !graphics_changed {
                continue;
            }
            let graphics = graphics.map(|(graphics, delivery, sources)| {
                let mut next_surface = surface.clone();
                crate::server::render_stream::apply_pane_surface_patch(&mut next_surface, &patch);
                next_surface.graphics = graphics;
                (next_surface, delivery, sources)
            });
            updates.push(RetainedRecipientUpdate {
                client_id,
                patch,
                graphics,
            });
        }
        // All recipients share the collection above: a second collection would consume
        // dirty rows already needed by another client, including clients of another size.
        let mut terminal_updates = Vec::with_capacity(terminal_recipients.len());
        for recipient in &terminal_recipients {
            let Some(collected_pane) = collected
                .iter()
                .find(|pane| pane.terminal_id.as_deref() == Some(recipient.terminal_id))
            else {
                continue;
            };
            if collected_pane.graphics_may_have_placements {
                fallback!("terminal_graphics");
            }
            // The ANSI patch encoder writes no OSC 8 cells; a complete frame does.
            if !collected_pane.patch.hyperlinks.is_empty() {
                fallback!("terminal_hyperlink");
            }
            if terminal_has_clipped_wide_edge(recipient.frame, Some(&collected_pane.patch)) {
                fallback!("terminal_clipped_wide_patch");
            }
            let area = protocol::SurfaceRect {
                x: 0,
                y: 0,
                width: recipient.frame.width,
                height: recipient.frame.height,
            };
            let Some(rows) = changed_rows(recipient.frame, area, &collected_pane.patch) else {
                fallback!("terminal_invalid_patch");
            };
            let Some(runtime) = self.runtime_for_terminal_id_string(recipient.terminal_id) else {
                fallback!("runtime_missing");
            };
            let cursor = runtime
                .cursor_state(Rect::new(0, 0, area.width, area.height), true)
                .map(|cursor| protocol::CursorState {
                    x: cursor.x,
                    y: cursor.y,
                    visible: cursor.visible
                        && collected_pane
                            .scroll_metrics
                            .is_none_or(|metrics| metrics.offset_from_bottom == 0),
                    shape: cursor.shape,
                });
            if rows.is_empty() && cursor == recipient.frame.cursor {
                continue;
            }
            let Some(prepared) = self.clients[&recipient.client_id]
                .render_state
                .prepare_terminal_patch(rows, cursor)
            else {
                fallback!("terminal_prepare");
            };
            terminal_updates.push((recipient.client_id, prepared));
        }
        if updates.is_empty() && terminal_updates.is_empty() {
            success!("unchanged");
        }
        if recipients
            .iter()
            .any(|recipient| has_synchronized_pane(&self.app, recipient.surface))
        {
            fallback!("synchronized_during_patch");
        }
        for collected_pane in &collected {
            let Some((workspace_index, pane_id)) = self.app.parse_pane_id(&collected_pane.pane_id)
            else {
                fallback!("pane_missing");
            };
            let Some(runtime) = self.app.state.runtime_for_pane_in_workspace(
                &self.app.terminal_runtimes,
                workspace_index,
                pane_id,
            ) else {
                fallback!("runtime_missing");
            };
            if runtime.synchronized_output_state() != (false, collected_pane.synchronized_epoch) {
                fallback!("synchronized_during_patch");
            }
        }

        let mut sent = 0u64;
        let mut deferred = 0u64;
        let mut disconnected = Vec::new();
        for update in updates {
            let RetainedRecipientUpdate {
                client_id,
                patch,
                mut graphics,
            } = update;
            if graphics.as_ref().is_some_and(|(surface, _, _)| {
                self.defer_changed_native_geometry(client_id, &surface.graphics)
            }) {
                deferred += 1;
                continue;
            }
            let native_upload = graphics.as_mut().and_then(|(surface, delivery, sources)| {
                self.prepare_native_scene(client_id, &mut surface.graphics, delivery, sources)
            });
            let Some(client) = self.clients.get_mut(&client_id) else {
                continue;
            };
            let Some(writer) = client.writer.as_ref().cloned() else {
                client.defer_full_render();
                deferred += 1;
                continue;
            };
            // The published row patch cannot carry images. Reuse the retained text/layout
            // in a graphics-capable surface message rather than invoking the full renderer.
            let (prepared, graphics_delivery) = if let Some((surface, delivery, _)) = graphics {
                (
                    client
                        .render_state
                        .prepare_pane_surface_with_file(surface, native_upload.is_some()),
                    Some(delivery),
                )
            } else {
                (client.render_state.prepare_pane_surface_patch(patch), None)
            };
            let Some(prepared) = prepared else {
                client.defer_full_render();
                deferred += 1;
                continue;
            };
            let max_frame_size = if graphics_delivery.is_some() {
                MAX_GRAPHICS_FRAME_SIZE
            } else {
                protocol::MAX_FRAME_SIZE
            };
            let mut serialized =
                match Self::frame_server_message_with_max(prepared.message(), max_frame_size) {
                    Ok(serialized) => serialized,
                    Err(error) => {
                        warn!(
                            client_id,
                            %error,
                            "failed to serialize retained pane surface patch"
                        );
                        // A delta may own an encoded graphics payload that cannot be
                        // trimmed in place. Force the bounded full-surface recovery path.
                        client.render_state.request_repaint();
                        client.defer_full_render();
                        deferred += 1;
                        continue;
                    }
                };
            if let Some((_, message)) = &native_upload {
                let Ok(file_frame) =
                    Self::frame_server_message_with_max(message, MAX_GRAPHICS_FRAME_SIZE)
                else {
                    client.defer_full_render();
                    deferred += 1;
                    continue;
                };
                serialized.extend_from_slice(&file_frame);
            }
            crate::render_prof::counter("retained_surface.bytes", serialized.len() as u64);
            let send = if native_upload.is_some() || self.native_graphics.is_pending(client_id) {
                writer.render.send_ordered(serialized)
            } else {
                writer.render.try_send(serialized)
            };
            match send {
                Ok(()) => {
                    if let Some((graphics, inline_assets)) = prepared.queued_surface_graphics() {
                        self.native_graphics
                            .commit_scene(client_id, graphics, inline_assets);
                    }
                    if let Some((pending, _)) = native_upload {
                        self.native_graphics.commit(client_id, pending);
                    }
                    let graphics_pending = graphics_delivery
                        .as_ref()
                        .is_some_and(crate::kitty_graphics::surface::DeliveryCache::has_pending);
                    if let Some(delivery) = graphics_delivery {
                        client.shell_graphics_delivery = delivery;
                    }
                    if graphics_pending {
                        client.defer_full_render();
                    } else {
                        client.clear_deferred_render();
                    }
                    client.render_state.commit_sent_frame(prepared);
                    sent += 1;
                }
                Err(std::sync::mpsc::TrySendError::Full(_)) => {
                    client.defer_full_render();
                    deferred += 1;
                }
                Err(std::sync::mpsc::TrySendError::Disconnected(_)) => {
                    disconnected.push(client_id);
                }
            }
        }
        for (client_id, prepared) in terminal_updates {
            let Some(client) = self.clients.get_mut(&client_id) else {
                continue;
            };
            let Some(writer) = client.writer.as_ref() else {
                client.defer_full_render();
                deferred += 1;
                continue;
            };
            let serialized = match Self::frame_server_message(prepared.message()) {
                Ok(serialized) => serialized,
                Err(error) => {
                    warn!(client_id, %error, "failed to serialize retained terminal patch");
                    client.render_state.request_repaint();
                    client.defer_full_render();
                    deferred += 1;
                    continue;
                }
            };
            crate::render_prof::counter("retained_surface.bytes", serialized.len() as u64);
            match writer.render.try_send(serialized) {
                Ok(()) => {
                    client.render_state.commit_sent_frame(prepared);
                    client.clear_deferred_render();
                    sent += 1;
                }
                Err(std::sync::mpsc::TrySendError::Full(_)) => {
                    client.defer_full_render();
                    deferred += 1;
                }
                Err(std::sync::mpsc::TrySendError::Disconnected(_)) => disconnected.push(client_id),
            }
        }
        for client_id in disconnected {
            self.remove_client_and_resize_if_needed(client_id);
        }
        crate::render_prof::counter("retained_surface.recipients.sent", sent);
        crate::render_prof::counter("retained_surface.recipients.deferred", deferred);
        if sent > 0 {
            success!("sent");
        }
        success!("recovery_queued");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(symbol: &str) -> protocol::CellData {
        protocol::CellData {
            symbol: symbol.to_owned(),
            fg: 0,
            bg: 0,
            modifier: 0,
            skip: false,
            hyperlink: None,
        }
    }

    #[test]
    fn terminal_clipped_wide_edge_uses_ansi_grapheme_width() {
        let voiced = cell("ｶ\u{ff9e}");
        assert_eq!(
            unicode_width::UnicodeWidthStr::width(voiced.symbol.as_str()),
            1
        );
        assert_eq!(crate::protocol::render_ansi::cell_width(&voiced), 2);
        let mut frame = FrameData {
            width: 2,
            height: 1,
            cells: vec![cell(" "), voiced.clone()],
            cursor: None,
            hyperlinks: Vec::new(),
            graphics: Vec::new(),
        };
        assert!(terminal_has_clipped_wide_edge(&frame, None));
        frame.cells[1] = cell(" ");
        assert!(!terminal_has_clipped_wide_edge(&frame, None));
        let patch = crate::pane::TerminalDirtyPatch {
            rows: vec![(0, vec![cell(" "), voiced])],
            hyperlinks: Vec::new(),
        };
        assert!(terminal_has_clipped_wide_edge(&frame, Some(&patch)));
    }

    #[test]
    fn retained_rows_send_only_changed_cell_spans() {
        let frame = FrameData {
            width: 6,
            height: 2,
            cells: vec![cell(" "); 12],
            cursor: None,
            hyperlinks: Vec::new(),
            graphics: Vec::new(),
        };
        let patch = crate::pane::TerminalDirtyPatch {
            rows: vec![(0, vec![cell(" "), cell("x"), cell("y"), cell(" ")])],
            hyperlinks: Vec::new(),
        };

        let rows = changed_rows(
            &frame,
            protocol::SurfaceRect {
                x: 1,
                y: 1,
                width: 4,
                height: 1,
            },
            &patch,
        )
        .expect("valid patch");

        assert_eq!(
            rows,
            vec![protocol::PaneSurfacePatchRow {
                x: 2,
                y: 1,
                cells: vec![cell("x"), cell("y"), cell(" ")],
            }]
        );
        assert_eq!(frame.cells, vec![cell(" "); 12], "planning must not commit");
    }

    #[test]
    fn retained_rows_include_the_cell_after_a_width_transition() {
        let frame = FrameData {
            width: 3,
            height: 1,
            cells: vec![cell("界"), cell("z"), cell("q")],
            cursor: None,
            hyperlinks: Vec::new(),
            graphics: Vec::new(),
        };
        let patch = crate::pane::TerminalDirtyPatch {
            rows: vec![(0, vec![cell("x"), cell("z"), cell("q")])],
            hyperlinks: Vec::new(),
        };

        let rows = changed_rows(
            &frame,
            protocol::SurfaceRect {
                x: 0,
                y: 0,
                width: 3,
                height: 1,
            },
            &patch,
        )
        .expect("valid patch");

        assert_eq!(
            rows,
            vec![protocol::PaneSurfacePatchRow {
                x: 0,
                y: 0,
                cells: vec![cell("x"), cell("z")],
            }]
        );
    }

    #[test]
    fn retained_rows_omit_unchanged_full_dirty_rows() {
        let frame = FrameData {
            width: 4,
            height: 2,
            cells: vec![cell(" "); 8],
            cursor: None,
            hyperlinks: Vec::new(),
            graphics: Vec::new(),
        };
        let patch = crate::pane::TerminalDirtyPatch {
            rows: vec![(0, vec![cell(" "); 4]), (1, vec![cell(" "); 4])],
            hyperlinks: Vec::new(),
        };

        let rows = changed_rows(
            &frame,
            protocol::SurfaceRect {
                x: 0,
                y: 0,
                width: 4,
                height: 2,
            },
            &patch,
        )
        .expect("valid patch");

        assert!(rows.is_empty());
    }
}
