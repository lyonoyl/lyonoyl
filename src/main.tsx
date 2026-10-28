import React, { type ReactNode } from "react";
import ReactDOM from "react-dom/client";
import { Direction } from "radix-ui";
import App from "./App";
import { NativeWords } from "@/app/native-words";
import { I18nProvider, useI18n } from "@/shared/i18n";
import { systemLanguages } from "@/shared/ipc/client";
import "./index.css";

/**
 * Tells the menus, lists and pickers which way the window runs. They read it from here rather than
 * from the page: without it a submenu opens over the window's edge in a mirrored window and the
 * arrow keys move the wrong way through it.
 */
function Directed({ children }: { children: ReactNode }) {
  return (
    <Direction.Provider dir={useI18n().dir}>{children}</Direction.Provider>
  );
}

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <I18nProvider askTheSystem={systemLanguages}>
      <Directed>
        <NativeWords />
        <App />
      </Directed>
    </I18nProvider>
  </React.StrictMode>,
);
