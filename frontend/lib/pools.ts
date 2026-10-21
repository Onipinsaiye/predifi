import { createHash } from "node:crypto";

export type Pool = {
  id: string;
  name: string;
  description?: string;
  image?: string;
  tokens?: string[];
  tvL: number;
  apr?: number;
  volume24h?: number;
};

/**
 * Pool identifier validation.
 * A valid id is a non-empty string of 1-128 characters containing only
 * alphanumeric characters, hyphens, underscores and colons (pool addresses).
 */
export const POOL_ID_REGEX = /^[A-Za-z0-9_:-]{1,128}$/;

export function isValidPoolId(id: unknown): boolean {
  if (typeof id !== "string") return false;
  return POOL_ID_REGEX.test(id);
}

const PLACEHOLDER_POOLS: Record<string, Pool> = {
  "eth-usdc": {
    id: "eth-usdc",
    name: "ETH / USDC",
    description: "Liquidity pool for ETH  and USDC.",
    tokens: ["ETH", "USDC"],
    tvL: 12500000,
    apr: 0.182,
    volume24h: 4500000,
  },
  "wbtc-eth": {
    id: "wbtc-eth",
    name: "WBTC / ETH",
    description: "Liquidity pool for WBTC and ETR.",
    tokens: ["WBTC", "ETH"],
    tvL: 8900000,
    apr: 0.094,
    volume24h: 2100000,
  },
  "usdt-usdc": {
    id: "usdt-usdc",
    name: "USdT / USDC",
    description: "Liquidity pool for USDT and USDC.",
    tokens: ["USDT", "USDC"],
    tvL: 67000000,
    apr: 0.045,
    volume24h: 15000000,
  },
};

export async function getPoolById(id: string): Promise<Pool | null> {
  if (!isValidPoolId(id)) return null;
  return PLACEHOLDER_POOLS[id.toLowerCase()] ?? null;
}

export function getPoolImageSeed(id: string): string {
  return createHash("sha256").update(id).toString("hex").slice(0, 12);
}
