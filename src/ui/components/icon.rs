#[derive(Debug, Clone)]
pub struct LocalIcon {}

impl LocalIcon {}

#[derive(Debug, Clone)]
pub struct UrlIcon {}

impl UrlIcon {}

#[derive(Debug, Clone)]
pub enum Icon {
    Local(LocalIcon),
    FromUrl(UrlIcon),
}
