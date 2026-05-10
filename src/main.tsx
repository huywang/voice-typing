import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import FloatingBar from "./FloatingBar";

// Check the URL query string to decide which view to render.
// The "floating-bar" Tauri window loads "index.html?view=floating".
const params = new URLSearchParams(window.location.search);
const isFloating = params.get("view") === "floating";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    {isFloating ? <FloatingBar /> : <App />}
  </React.StrictMode>,
);
