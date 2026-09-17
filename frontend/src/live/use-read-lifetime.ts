import { useEffect } from "react";
import { useQueryClient, type QueryKey } from "@tanstack/react-query";
import { discardRead } from "./read-cache.ts";

/** Release this component's exact read on navigation/unmount, including pending work. */
export function useReadLifetime(key: QueryKey) {
  const queries = useQueryClient();
  useEffect(() => () => discardRead(queries, key), [queries, key]);
}
