import { defineConfig } from "vite";
import { randomBytes } from "node:crypto";
import path from "node:path";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

/**
 * What lets the development server's own inline script run, and nothing else.
 *
 * The React plugin injects one inline module into the page — the preamble Fast Refresh needs — and
 * the policy below refuses inline scripts exactly as the shipped one does. Teaching development to
 * accept `'unsafe-inline'` would make it a poor rehearsal for what ships: an inline script written
 * by mistake would work here and fail in front of a user. A nonce lets that one line through and
 * still refuses every other, so the two builds behave the same way about the thing that matters.
 *
 * New at every start, because a value that never changes is a value an attacker can write down.
 */
const NONCE = randomBytes(16).toString("base64");

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [react(), tailwindcss()],

  // Stamped onto the tags Vite injects itself, so the policy above can name them without naming
  // every inline script there might ever be.
  html: { cspNonce: NONCE },

  test: {
    // What makes React lose a race it is allowed to win, so an ordering a person meets
    // occasionally is one this suite meets always. `src/tests-lose-the-race.ts` says why.
    setupFiles: ["./src/tests-lose-the-race.ts"],
    // happy-dom rather than jsdom: jsdom 30 needs a newer Node than the toolchain pins.
    environment: "happy-dom",
    // The reading pane puts a message in an iframe on a scheme only the application serves. In a
    // browser that is the whole security design; here there is nothing behind `lyomsg://`, so
    // happy-dom tries to fetch it, fails, and raises the failure asynchronously — long after the
    // test that rendered the frame has finished. Whichever test happens to be running then fails,
    // in a different file each run, about something it never did. Tests assert on the frame's
    // attributes, never on what is inside it, so not loading it costs nothing.
    environmentOptions: {
      happyDOM: { settings: { disableIframePageLoading: true } },
    },
    globals: true,
    include: ["src/**/*.{test,spec}.{ts,tsx}", "scripts/**/*.test.mjs"],
  },

  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    // The window opens on a page that has to be built out of fifty modules, and building them
    // when the window asks is what a person sees as several seconds of white. Measured here:
    // ten seconds cold, a tenth of a second once they are built.
    //
    // Building them at start instead costs nothing at all in this workflow: `tauri dev` spends
    // twenty seconds compiling Rust before there is a window to ask, and this happens inside it.
    warmup: { clientFiles: ["./src/main.tsx"] },
    // The application's own policy, sent by the development server.
    //
    // In a built application Tauri sends it with the page. In development the page comes from here
    // instead, and Tauri's copy is never delivered — so without this line the fourth layer around a
    // message (`frame-src`, which says a message may only be framed from its own scheme) is absent
    // in exactly the build the application is worked on every day.
    headers: {
      "Content-Security-Policy":
        `default-src 'none'; script-src 'self' 'nonce-${NONCE}'; style-src 'self' 'unsafe-inline'; style-src-elem 'self' 'unsafe-inline'; style-src-attr 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self' data:; connect-src 'self' ipc: http://ipc.localhost ws://localhost:1420 http://localhost:1420; media-src 'self'; worker-src 'self' blob:; frame-src lyomsg: http://lyomsg.localhost; object-src 'none'; base-uri 'none'; form-action 'none'`,
    },
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },
}));
