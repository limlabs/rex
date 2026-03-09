import React from "react";
import Sidebar from "../components/Sidebar";
import "../styles/globals.css";

export const metadata = {
  title: {
    default: "Rex Documentation",
    template: "%s | Rex Docs",
  },
  description:
    "Documentation for Rex — a Rust-native React framework with Next.js compatibility.",
};

export default function RootLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <html lang="en">
      <body className="bg-white text-slate-900 antialiased">
        <Sidebar />
        <main className="lg:pl-64 min-h-screen">
          <div className="max-w-3xl mx-auto px-6 py-12 lg:px-8">
            <article className="prose">{children}</article>
          </div>
        </main>
      </body>
    </html>
  );
}
