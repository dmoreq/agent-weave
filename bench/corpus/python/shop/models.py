class Customer:
    def __init__(self, email: str) -> None:
        self.email = email


class Order:
    def __init__(self, customer: Customer, amount: int) -> None:
        self.customer = customer
        self.amount = amount

    def total(self) -> int:
        return self.amount

    def discount(self) -> int:
        return 0


class DiscountedOrder(Order):
    def discount(self) -> int:
        return self.total() // 10
