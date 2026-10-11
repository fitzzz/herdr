use crate::app;

pub(crate) fn app_keybindings(app: &app::App) -> crate::config::LiveKeybindConfig {
    crate::config::LiveKeybindConfig {
        prefix: app.state.prefix_keys.clone(),
        keybinds: app.state.keybinds.clone(),
    }
}

pub(crate) fn apply_keybindings(
    app: &mut app::App,
    keybindings: &crate::config::LiveKeybindConfig,
) {
    // Explicit config restoration only. Foreground presentation synchronization
    // must not copy the unchanged server keymap on each API read or input event.
    app.state.prefix_keys.clone_from(&keybindings.prefix);
    app.state.keybinds = keybindings.keybinds.clone();
}
