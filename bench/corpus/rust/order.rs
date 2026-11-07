use crate::pricing;

pub struct Order {
    amount: u64,
}

impl Order {
    pub fn new(amount: u64) -> Self {
        Self { amount }
    }

    pub fn checkout(&self) -> u64 {
        pricing::apply_discount(self.amount)
    }

    pub fn total(&self) -> u64 {
        pricing::total(self.amount)
    }
}
