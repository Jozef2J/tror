use super::*;

impl TrorApp {
    pub(super) fn palette(&self) -> Palette {
        self.settings.look.palette()
    }

    pub(super) fn apply_look(&mut self, ctx: &egui::Context) {
        theme::commit_term_fonts();
        let wanted_fonts = self.console.fonts();
        let fonts_changed = self.term_fonts.as_ref() != Some(&wanted_fonts);
        if self.applied.as_ref() == Some(&self.settings.look) && !fonts_changed {
            return;
        }
        let look = self.settings.look.clone();
        let old = self.applied.take();
        let fonts = |l: &Look| (l.ui_font.clone(), l.code_font.clone(), l.tab_width);
        if (old.as_ref().map(fonts) != Some(fonts(&look)) || fonts_changed)
            && let Err(e) = theme::install_fonts(
                ctx,
                look.ui_font.as_deref(),
                look.code_font.as_deref(),
                &wanted_fonts,
                look.tab_width,
            )
        {
            self.notify(format!("Шрифт не загружен: {e}"), ctx.input(|i| i.time));
        }
        self.term_fonts = Some(wanted_fonts);
        theme::apply(ctx, &look.palette(), look.dark, look.radius, look.ui_size);
        let code = |l: &Look| {
            let (scheme, colors) = l.syntax();
            (scheme.to_owned(), colors.clone())
        };
        if old.as_ref().map(code) != Some(code(&look)) {
            let (scheme, colors) = look.syntax();
            self.syntaxes.set_theme(scheme, colors);
        }
        self.applied = Some(look);
    }
}
