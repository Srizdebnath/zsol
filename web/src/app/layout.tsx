import type { Metadata } from "next";
import Link from "next/link";
import "./globals.css";

export const metadata: Metadata = {
  title: "zsol — agent solvency attestations on Zcash",
  description:
    "Live, private proof of funds for AI agents on Zcash, and a Credit Desk that turns it into pay-later Bitrefill purchases.",
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
          <Link href="/desk">credit desk</Link>
        </nav>
        <main>{children}</main>
        <div className="footnote" style={{ maxWidth: 980, margin: "0 auto 32px" }}>
          Every value on these pages is fetched live from the registry, the MCP gateway or the
          Credit Desk, which read Zcash testnet. A service that is down shows DOWN.
        </div>
      </body>
    </html>
  );
}
