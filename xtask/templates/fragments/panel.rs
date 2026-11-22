
#[derive(Default)]
struct {{Ident}}Panel {
    clicks: u32,
}

impl Panel for {{Ident}}Panel {
    fn ui(&mut self, ui: &mut Ui) {
        if ui.button("Count") {
            self.clicks += 1;
        }
        ui.label(&format!("{} clicks", self.clicks));
    }
}
