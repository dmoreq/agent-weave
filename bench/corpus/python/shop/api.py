from shop.models import Customer
from shop.service import apply_discount, create_order


def handle_request(email: str, amount: int) -> int:
    customer = Customer(email)
    order = create_order(customer, amount)
    return apply_discount(order)
