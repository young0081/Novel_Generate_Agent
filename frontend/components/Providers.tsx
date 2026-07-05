"use client";

import type { ReactNode } from "react";

import { ConnectionProvider } from "@/components/Connection";
import { ToastProvider } from "@/components/Toast";

/** Single client-side wrapper that supplies toasts + connection state app-wide. */
export default function Providers({ children }: { children: ReactNode }) {
  return (
    <ToastProvider>
      <ConnectionProvider>{children}</ConnectionProvider>
    </ToastProvider>
  );
}
