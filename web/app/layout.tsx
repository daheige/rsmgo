import type { Metadata } from "next";
import "./globals.css";

export const metadata: Metadata = {
  title: "rsmgo - Model-Agnostic AI Agent",
  description: "Universal AI agent infrastructure",
};

export default function RootLayout({
  children,
}: Readonly<{
  children: React.ReactNode;
}>) {
  return (
    <html lang="zh-CN">
      <body>{children}</body>
    </html>
  );
}
