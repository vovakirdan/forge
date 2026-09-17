import { createRoot } from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { LiveApp } from "./LiveApp.tsx";
import { createLiveApi } from "./api.ts";
import { createLiveSession } from "./session.ts";
import "../styles.css";

const queries = new QueryClient({
  defaultOptions: {
    queries: {
      retry: false,
      refetchOnWindowFocus: false,
      refetchOnReconnect: false,
      networkMode: "always",
    },
    mutations: { retry: false },
  },
});
const api = createLiveApi();
const session = createLiveSession(api, queries);
const root = document.getElementById("root");
if (!root) throw new Error("Forge root element is missing");
createRoot(root).render(
  <QueryClientProvider client={queries}>
    <LiveApp api={api} session={session} />
  </QueryClientProvider>,
);
