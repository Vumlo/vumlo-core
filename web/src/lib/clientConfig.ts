import type { ClientConfig } from "@/generated/client-config";
import raw from "@/generated/client-config.json";

/** What `vumlo-cli brand apply` generated from vumlo.toml: the public keys, and nothing private. */
export const clientConfig = raw as unknown as ClientConfig;
