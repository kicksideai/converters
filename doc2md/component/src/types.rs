#[derive(Debug, Clone)]
pub struct Image {
    pub name: String,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Default)]
pub struct Converted {
    pub markdown: String,
    pub images: Vec<Image>,
}
