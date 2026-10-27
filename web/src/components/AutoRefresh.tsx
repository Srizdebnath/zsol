"use client";

// Re-runs the server component's live fetches on an interval (router.refresh keeps
// client state and re-requests the RSC payload; nothing is cached).
import { useRouter } from "next/navigation";
import { useEffect, useState } from "react";

export function AutoRefresh({ seconds }: { seconds: number }) {
  const router = useRouter();
  const [at, setAt] = useState<string>("");
  useEffect(() => {
    setAt(new Date().toLocaleTimeString());
    const t = setInterval(() => {
      router.refresh();
      setAt(new Date().toLocaleTimeString());
    }, seconds * 1000);
    return () => clearInterval(t);
  }, [router, seconds]);
  return (
    <small>
      re-fetched every {seconds}s{at ? ` · last ${at}` : ""}
    </small>
  );
}
