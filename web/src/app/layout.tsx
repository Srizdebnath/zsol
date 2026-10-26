import type { Metadata } from "next";
import Link from "next/link";
import "./globals.css";

export const metadata: Metadata = {
  title: "zsol — agent solvency attestations on Zcash",
  description:
    "Proof-of-funds attestations for AI agents: shielded self-spend anchors, digest-chained claim registry, MCP gateway.",
};

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en">
      <body>
        <nav className="nav">
          <strong style={{ padding: "6px 4px" }}>zsol</strong>
          <Link href="/">status</Link>
          <Link href="/explorer">explorer</Link>
          <Link href="/verify">verify</Link>
        </nav>
        <main>{children}</main>
        <div className="footnote" style={{ maxWidth: 980, margin: "0 auto 32px" }}>
          zsol is evidence ABOUT claims, never a source of truth for solvency. Everything this
          page shows is fetched live from the registry / MCP gateway; down services show DOWN.
        </div>
      </body>
    </html>
  );
}
