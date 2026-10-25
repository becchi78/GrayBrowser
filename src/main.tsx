import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { ReachabilityProvider } from "./components/ReachabilityProvider";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <ReachabilityProvider>
      <App />
    </ReachabilityProvider>
  </React.StrictMode>,
);
