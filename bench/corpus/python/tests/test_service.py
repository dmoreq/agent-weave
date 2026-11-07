from shop.models import Customer
from shop.service import create_order


def test_create_order() -> None:
    order = create_order(Customer("buyer@example.com"), 10)
    assert order is not None
