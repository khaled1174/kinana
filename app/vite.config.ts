import { fileURLToPath, URL } from "node:url";
import { readFileSync } from "node:fs";
import { defineConfig, type Plugin } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

function cspProbe():Plugin {return {name:"e2e-only-csp-probe",generateBundle(){this.emitFile({type:"asset",fileName:"e2e-csp-self-probe.js",source:readFileSync(fileURLToPath(new URL("./src/e2e-csp-self-probe.txt",import.meta.url)),"utf8")});}};}
export default defineConfig(({ mode }) => ({
  resolve: { alias: { "@": fileURLToPath(new URL("./src", import.meta.url)) } },
  plugins: [react(), tailwindcss(), ...(mode === "e2e" ? [cspProbe()]:[])],
  build: { target: "safari16.4", outDir: mode === "e2e" ? "dist-e2e" : "dist", emptyOutDir: true },
  clearScreen: false,
}));
