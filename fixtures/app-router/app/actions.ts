"use server";

export async function incrementCounter(current: number): Promise<number> {
  return current + 1;
}

export async function decrementCounter(current: number): Promise<number> {
  return current - 1;
}
