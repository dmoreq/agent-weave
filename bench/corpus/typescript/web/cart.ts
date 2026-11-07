export class Cart {
  private items: number[] = [];

  add(item: number): number {
    this.items.push(item);
    return this.total();
  }

  total(): number {
    return this.items.reduce((sum, item) => sum + item, 0);
  }
}
