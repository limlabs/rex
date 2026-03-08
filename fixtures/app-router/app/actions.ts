"use server";

import { redirect, notFound, headers } from "rex/actions";

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

export async function redirectToHome(): Promise<void> {
  redirect("/");
}

export async function requireItem(id: string): Promise<{ id: string }> {
  if (id === "missing") {
    notFound();
  }
  return { id };
}

export async function echoUserAgent(): Promise<string> {
  const h = headers();
  return h["user-agent"] || "unknown";
}
