import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import App from "@renderer/App";

import "@renderer/assets/main.css";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
