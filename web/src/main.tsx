import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { BrowserRouter } from "react-router";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

import { App } from "./App";
import "./styles/index.css";

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      // The server owns essentially all state here, so refetching on window
      // focus is mostly wasted requests against data that rarely changes
      // out from under the user.
      refetchOnWindowFocus: false,
      // A 401 means "sign in", not "try again" — retrying it just delays the
      // redirect. Anything else gets one retry.
      retry: (failureCount, error) =>
        !(error instanceof Response && error.status === 401) && failureCount < 1,
    },
  },
});

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      <BrowserRouter>
        <App />
      </BrowserRouter>
    </QueryClientProvider>
  </StrictMode>,
);
