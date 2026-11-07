from shop.models import Customer, DiscountedOrder, Order


def create_order(customer: Customer, amount: int) -> DiscountedOrder:
    return DiscountedOrder(customer, amount)


def apply_discount(order: Order) -> int:
    return order.discount()
