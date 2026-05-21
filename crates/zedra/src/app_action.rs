use gpui::Action;

#[derive(Clone, PartialEq, Action)]
#[action(namespace = app, no_json)]
pub struct SystemBack;
