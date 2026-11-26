use super::*;

impl TrorApp {
    pub(super) fn shortcuts(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        if self.capturing.is_some() {
            return;
        }
        let keys = self.settings.shortcuts();
        let weight = crate::keys::weight;
        let mut extra: Vec<(egui::KeyboardShortcut, Action)> = Vec::new();
        for (index, plugin) in self.plugins.list.iter().enumerate() {
            if let Some(shortcut) = crate::keys::parse(&plugin.shortcut) {
                extra.push((shortcut, Action::Plugin(index)));
            }
        }
        for (index, plugin) in self.plugins.wasm.iter().enumerate() {
            for (command, def) in plugin.manifest.commands.iter().enumerate() {
                if let Some(shortcut) = crate::keys::parse(&def.key) {
                    extra.push((shortcut, Action::PluginCommand(index, command)));
                }
            }
        }
        extra.retain(|(shortcut, _)| crate::keys::usable(shortcut));
        extra.sort_by_key(|(shortcut, _)| std::cmp::Reverse(weight(shortcut)));
        let mut extra = extra.into_iter().peekable();
        for (index, shortcut) in keys.bound() {
            while let Some((next, _)) = extra.peek()
                && weight(next) > weight(shortcut)
            {
                if let Some((next, action)) = extra.next()
                    && ctx.input_mut(|i| crate::keys::consume(i, &next))
                {
                    actions.push(action);
                }
            }
            let (id, _, _, editing) = crate::keys::COMMANDS[*index];
            if self.console.focused() && editing {
                continue;
            }
            if !ctx.input_mut(|i| crate::keys::consume(i, shortcut)) {
                continue;
            }
            if shortcut.modifiers.alt {
                ctx.input_mut(|i| i.events.retain(|e| !matches!(e, egui::Event::Text(_))));
            }
            actions.push(self.command(id));
        }
        for (shortcut, action) in extra {
            if ctx.input_mut(|i| crate::keys::consume(i, &shortcut)) {
                actions.push(action);
            }
        }
        if self.search.open
            && self.goto.is_none()
            && self.confirm.is_none()
            && !self.complete.open
            && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))
        {
            actions.push(Action::CloseSearch);
        }
    }
}
