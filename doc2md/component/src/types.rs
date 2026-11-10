#[derive(Debug, Clone)]
pub struct Image {
    pub name: String,
    pub data: Vec<u8>,
    pub page: Option<u32>,
}

#[derive(Debug, Clone, Default)]
pub struct Converted {
    pub markdown: String,
    pub images: Vec<Image>,
}
