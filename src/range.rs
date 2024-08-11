use std::fmt;

#[derive(Copy, Clone)]
pub struct Range {
    pub min: i16,
    pub max: i16
}

impl Range {
    pub(crate) fn new(min: i16, max: i16) -> Self {
        Self { min, max }
    }

    pub(crate) fn transpose(&mut self, other: Range) {
        self.min = self.min.max(other.min);
        self.max = self.max.min(other.max);
    }
}

impl fmt::Display for Range {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}ms, {}ms", self.min, self.max)
    }
}