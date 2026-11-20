import React from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import "./styles.css";

async function start() {
  if (import.meta.env.MODE === "e2e") {
    await import("@wdio/tauri-plugin");
    (await import("./e2e-csp")).installCspProbe();
  }
  createRoot(document.getElementById("root")!).render(<React.StrictMode><App /></React.StrictMode>);
}
void start();
