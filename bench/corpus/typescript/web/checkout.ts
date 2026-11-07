import { Cart } from "./cart";

export function checkout(cart: Cart): number {
  return cart.add(10);
}
