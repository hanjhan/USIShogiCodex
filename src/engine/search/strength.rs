#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchStrength {
    Weak,
    Normal,
    Strong,
}

impl SearchStrength {
    pub fn depth(self) -> u8 {
        match self {
            SearchStrength::Weak => 10,
            SearchStrength::Normal => 12,
            SearchStrength::Strong => 14,
        }
    }

    pub fn describe(self) -> &'static str {
        match self {
            SearchStrength::Weak => "Weak",
            SearchStrength::Normal => "Normal",
            SearchStrength::Strong => "Strong",
        }
    }
}

impl Default for SearchStrength {
    fn default() -> Self {
        SearchStrength::Normal
    }
}
