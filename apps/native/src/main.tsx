import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./index.css";

// The overlay bar is always dark (Glass parity). Pinned statically —
// ThemeProvider's "d" key-toggle would be a footgun in a hotkey app.
document.documentElement.classList.add("dark");

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
