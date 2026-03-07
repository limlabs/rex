"use server";

export async function incrementCounter(current: number): Promise<number> {
  return current + 1;
}

export async function decrementCounter(current: number): Promise<number> {
  return current - 1;
}

export async function submitForm(formData: FormData): Promise<{ message: string }> {
  const name = formData.get("name") as string;
  return { message: `Hello, ${name || "anonymous"}!` };
}
