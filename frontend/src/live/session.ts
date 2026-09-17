import type { QueryClient } from "@tanstack/react-query";
import { describeApiError, LiveApiError, SessionSchema } from "./api.ts";
import type { LiveApi, SessionCredentials } from "./api.ts";

export const SESSION_STORAGE_KEY = "forge.live.session.v1";
type SessionStorage = Pick<Storage, "getItem" | "setItem" | "removeItem">;
type SessionSnapshot = {
  status: "logged_out" | "authenticating" | "authenticated" | "storage_unavailable";
  generation: number;
  notice: string | null;
};
const storageNotice =
  "Session storage is unavailable. Enable session storage and reload to connect.";

export function createLiveSession(
  api: LiveApi,
  queries: QueryClient,
  getStorage: () => SessionStorage = () => window.sessionStorage,
) {
  let credentials: SessionCredentials | null = null;
  let storage: SessionStorage | null = null;
  let snapshot: SessionSnapshot = { status: "logged_out", generation: 0, notice: null };
  let timer: ReturnType<typeof setTimeout> | undefined;
  let exchange: AbortController | undefined;
  const listeners = new Set<() => void>();

  function publish(status: SessionSnapshot["status"], notice: string | null = null) {
    snapshot = { ...snapshot, status, notice };
    for (const listener of listeners) listener();
  }

  function clear(notice: string | null = null) {
    credentials = null;
    clearTimeout(timer);
    exchange?.abort();
    snapshot = { ...snapshot, generation: snapshot.generation + 1 };
    void queries.cancelQueries();
    queries.clear();
    try {
      if (!storage) throw new Error("Storage unavailable");
      storage.removeItem(SESSION_STORAGE_KEY);
      publish("logged_out", notice);
      return true;
    } catch {
      publish("storage_unavailable", storageNotice);
      return false;
    }
  }

  function scheduleExpiry() {
    clearTimeout(timer);
    if (!credentials) return;
    const remaining = Date.parse(credentials.expires_at) - Date.now();
    if (remaining <= 0) {
      clear("The session expired. Connect again.");
      return;
    }
    timer = setTimeout(scheduleExpiry, Math.min(remaining, 2_147_483_647));
  }

  try {
    storage = getStorage();
    const raw = storage.getItem(SESSION_STORAGE_KEY);
    if (raw !== null) {
      let value: unknown;
      try {
        value = JSON.parse(raw);
      } catch {
        value = null;
      }
      const parsed = SessionSchema.safeParse(value);
      if (parsed.success && Date.parse(parsed.data.expires_at) > Date.now()) {
        credentials = parsed.data;
        publish("authenticated");
        scheduleExpiry();
      } else clear("The saved session is invalid or expired. Connect again.");
    }
  } catch {
    publish("storage_unavailable", storageNotice);
  }

  return {
    getSnapshot: () => snapshot,
    subscribe(listener: () => void) {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    async connect(code: string) {
      if (snapshot.status === "storage_unavailable" || snapshot.status === "authenticating") return;
      if (!clear()) return;
      const generation = snapshot.generation;
      exchange = new AbortController();
      publish("authenticating");
      try {
        const issued = await api.exchange(code, exchange.signal);
        if (snapshot.generation !== generation) return;
        if (Date.parse(issued.expires_at) <= Date.now()) throw new LiveApiError("invalid_response");
        try {
          if (!storage) throw new Error("Storage unavailable");
          storage.setItem(SESSION_STORAGE_KEY, JSON.stringify(issued));
        } catch {
          clear();
          publish("storage_unavailable", storageNotice);
          void api.logout(issued.token, AbortSignal.timeout(10_000)).catch(() => undefined);
          return;
        }
        credentials = issued;
        publish("authenticated");
        scheduleExpiry();
      } catch (error) {
        if (snapshot.generation !== generation) return;
        publish(
          "logged_out",
          error instanceof LiveApiError && error.kind === "unauthorized"
            ? "Code was rejected or expired."
            : describeApiError(error),
        );
      }
    },
    async logout() {
      const old = credentials;
      clear();
      const generation = snapshot.generation;
      if (!old) return;
      try {
        await api.logout(old.token, AbortSignal.timeout(10_000));
      } catch {
        if (snapshot.generation === generation && snapshot.status === "logged_out") {
          publish(
            "logged_out",
            "Signed out locally. Server revocation could not be confirmed; the session may remain valid until expiry.",
          );
        }
      }
    },
    async request<T>(generation: number, operation: (token: string) => Promise<T>): Promise<T> {
      const current = credentials;
      if (current && Date.parse(current.expires_at) <= Date.now())
        clear("The session expired. Connect again.");
      if (!credentials || snapshot.generation !== generation)
        throw new DOMException("Session changed", "AbortError");
      try {
        const value = await operation(credentials.token);
        if (credentials && Date.parse(credentials.expires_at) <= Date.now())
          clear("The session expired. Connect again.");
        if (snapshot.generation !== generation)
          throw new DOMException("Session changed", "AbortError");
        return value;
      } catch (error) {
        if (snapshot.generation !== generation)
          throw new DOMException("Session changed", "AbortError");
        if (error instanceof LiveApiError && error.kind === "unauthorized")
          clear("The session is no longer authorized. Connect again.");
        throw error;
      }
    },
    dispose() {
      clearTimeout(timer);
      exchange?.abort();
      listeners.clear();
    },
  };
}
export type LiveSession = ReturnType<typeof createLiveSession>;
