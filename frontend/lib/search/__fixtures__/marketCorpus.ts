import type { SearchableMarket } from "../marketSearch";

/**
 * Comprehensive, representative market corpus for prediction markets.
 * Covers diverse topics, states, creators, tags, stakes, and edge cases.
 */
export const REPRESENTATIVE_MARKET_CORPUS: SearchableMarket[] = [
  {
    pool_id: 101,
    name: "Bitcoin to surpass $100k in 2026",
    description:
      "Resolves Yes if Bitcoin (BTC) spot price exceeds $100,000 USD on Coinbase before Dec 31, 2026.",
    tags: ["crypto", "btc", "bitcoin", "finance"],
    creator: "GBTCMASTER777EXACTCREATOR1111111111111111111111111111111",
    state: "active",
    category: "Crypto",
    total_stake: 150000,
    created_at: "2026-01-10T12:00:00Z",
    end_time: 1798761600,
    token: "USDC",
  },
  {
    pool_id: 102,
    name: "Ethereum Dencun Upgrade final throughput exceeds 1000 TPS",
    description:
      "Evaluates Layer 2 blob-carrying transaction throughput metrics following protocol upgrade.",
    tags: ["crypto", "eth", "ethereum", "scaling"],
    creator: "GETHBUILDER22222222222222222222222222222222222222222222",
    state: "active",
    category: "Crypto",
    total_stake: 95000,
    created_at: "2026-02-01T15:30:00Z",
    end_time: 1780000000,
    token: "USDC",
  },
  {
    pool_id: 103,
    name: "Stellar Soroban smart contracts reach 10 million transactions",
    description: "Tracks total Soroban contract invocation volume across public mainnet ledgers.",
    tags: ["stellar", "soroban", "crypto", "smart-contracts"],
    creator: "GSTELLARDEV33333333333333333333333333333333333333333333",
    state: "active",
    category: "Crypto",
    total_stake: 220000,
    created_at: "2026-01-05T08:00:00Z",
    end_time: 1775000000,
    token: "XLM",
  },
  {
    pool_id: 104,
    name: "US Presidential Election 2028: Democratic nominee announced",
    description:
      "Prediction market on official candidate nomination at national political convention.",
    tags: ["politics", "usa", "election", "government"],
    creator: "GPOLITICSANALYSIS44444444444444444444444444444444444444",
    state: "active",
    category: "Politics",
    total_stake: 80000,
    created_at: "2026-03-01T10:00:00Z",
    end_time: 1850000000,
    token: "USDC",
  },
  {
    pool_id: 105,
    name: "FIFA World Cup 2026 Winner: Brazil vs Argentina final",
    description:
      "Championship final match prediction for the FIFA World Cup held in North America.",
    tags: ["sports", "football", "soccer", "fifa", "worldcup"],
    creator: "GSPORTSBETTOR555555555555555555555555555555555555555555",
    state: "closed",
    category: "Sports",
    total_stake: 120000,
    created_at: "2026-02-15T18:00:00Z",
    end_time: 1784000000,
    token: "USDC",
  },
  {
    pool_id: 106,
    name: "Champions League Winner 2026",
    description:
      "Settles on the official winner of UEFA Champions League European club football final.",
    tags: ["sports", "football", "uefa"],
    creator: "GSPORTSBETTOR555555555555555555555555555555555555555555",
    state: "settled",
    category: "Sports",
    total_stake: 60000,
    created_at: "2026-01-20T14:00:00Z",
    end_time: 1770000000,
    token: "USDC",
  },
  {
    pool_id: 107,
    name: "Artificial Intelligence AGI milestone reached by major lab",
    description:
      "Evaluates consensus benchmark score exceeding human parity across reasoning and mathematics.",
    tags: ["tech", "ai", "llm", "agi", "benchmark"],
    creator: "GRESEARCHER66666666666666666666666666666666666666666666",
    state: "active",
    category: "Tech",
    total_stake: 310000,
    created_at: "2026-02-20T09:15:00Z",
    end_time: 1790000000,
    token: "USDC",
  },
  {
    pool_id: 108,
    name: "Federal Reserve cuts interest rates below 3.5%",
    description:
      "Economic forecasting on FOMC rate decisions and macro monetary policy adjustments.",
    tags: ["economy", "macro", "fed", "interest-rates", "finance"],
    creator: "GMACROECONOMIST777777777777777777777777777777777777777",
    state: "active",
    category: "Economy",
    total_stake: 175000,
    created_at: "2026-01-28T16:45:00Z",
    end_time: 1782000000,
    token: "USDC",
  },
  {
    pool_id: 109,
    name: "Solana outages count in calendar year 2026",
    description:
      "Prediction on whether mainnet beta maintains 100% liveness without unplanned restarts.",
    tags: ["crypto", "solana", "sol", "infrastructure"],
    creator: "GINFRATESA8888888888888888888888888888888888888888888888",
    state: "closed",
    category: "Crypto",
    total_stake: 45000,
    created_at: "2026-01-02T11:00:00Z",
    end_time: 1798760000,
    token: "USDC",
  },
  {
    pool_id: 110,
    name: "Global average temperature breaks 2024 record",
    description:
      "Climate science data verification using Copernicus Climate Change Service annual summary.",
    tags: ["science", "climate", "environment", "temperature"],
    creator: "GCLIMATESCIENCE9999999999999999999999999999999999999999",
    state: "settled",
    category: "Science",
    total_stake: 52000,
    created_at: "2026-01-12T13:20:00Z",
    end_time: 1798761000,
    token: "USDC",
  },
];

// ---------------------------------------------------------------------------
// Dedicated Match Fixtures
// ---------------------------------------------------------------------------

/**
 * Fixture set for asserting exact matches across title, tags, creator, and state.
 */
export const EXACT_MATCH_FIXTURES: SearchableMarket[] = [
  {
    pool_id: 201,
    name: "Bitcoin",
    description: "General prediction on Bitcoin price trajectory.",
    tags: ["crypto", "btc"],
    creator: "GEXACTCREATOR111111111111111111111111111111111111111111",
    state: "active",
    total_stake: 10000,
  },
  {
    pool_id: 202,
    name: "Will Bitcoin hit new all-time high?",
    description: "Detailed analysis on Bitcoin ETF inflows and hash rate.",
    tags: ["bitcoin", "spot-etf"],
    creator: "GOTHERCREATOR222222222222222222222222222222222222222222",
    state: "closed",
    total_stake: 20000,
  },
  {
    pool_id: 203,
    name: "Ethereum staking yield exceeds 5%",
    description: "Consensus layer rewards and validator fee burn dynamics.",
    tags: ["ethereum", "defi"],
    creator: "GEXACTCREATOR111111111111111111111111111111111111111111",
    state: "settled",
    total_stake: 30000,
  },
];

/**
 * Fixture set for asserting partial matches: prefix, substring, and word boundaries.
 */
export const PARTIAL_MATCH_FIXTURES: SearchableMarket[] = [
  {
    pool_id: 301,
    name: "Stellar payment network volume", // starts with "Stellar" (prefix)
    description: "Cross-border remittance volume analysis.",
    tags: ["payments", "stellar-network"],
    creator: "GSTELLARDEV11111111111111111111111111111111111111111111",
    state: "active",
    total_stake: 50000,
  },
  {
    pool_id: 302,
    name: "Decentralized exchange volume includes Stellar assets", // "Stellar" inside (word exact)
    description: "Tracking automated market maker pairs.",
    tags: ["dex", "amm"],
    creator: "GAMMMANAGER2222222222222222222222222222222222222222222",
    state: "active",
    total_stake: 40000,
  },
  {
    pool_id: 303,
    name: "Interstellar travel milestone by 2050", // "stellar" as substring inside "Interstellar"
    description: "Aerospace technology milestones.",
    tags: ["space", "sci-fi"],
    creator: "GASTROPHYSICIST333333333333333333333333333333333333333",
    state: "active",
    total_stake: 30000,
  },
  {
    pool_id: 304,
    name: "Quantum computing supremacy benchmark",
    description: "Mentions stellar performance metrics in algorithmic error correction.", // "stellar" inside description only
    tags: ["quantum", "physics"],
    creator: "GQUANTUMLAB44444444444444444444444444444444444444444444",
    state: "active",
    total_stake: 20000,
  },
];

/**
 * Fixtures specifically designed with identical scores and match tiers to assert
 * that tie-break rules resolve deterministically.
 */
export const TIED_MATCH_FIXTURES: SearchableMarket[] = [
  // Pair 1: Tied on score and match tier, different total_stake
  {
    pool_id: 401,
    name: "Crypto Alpha Market",
    description: "Identical market structure A.",
    tags: ["alpha", "trading"],
    creator: "GCREATORALPHA1111111111111111111111111111111111111111",
    state: "active",
    total_stake: 5000, // lower stake
    created_at: "2026-01-01T00:00:00Z",
  },
  {
    pool_id: 402,
    name: "Crypto Beta Market",
    description: "Identical market structure B.",
    tags: ["alpha", "trading"],
    creator: "GCREATORALPHA1111111111111111111111111111111111111111",
    state: "active",
    total_stake: 15000, // higher stake -> wins tie-break
    created_at: "2026-01-01T00:00:00Z",
  },

  // Pair 2: Tied on score, match tier, and stake, different creation date
  {
    pool_id: 403,
    name: "Delta Prediction Protocol",
    description: "Shared description text for delta testing.",
    tags: ["delta"],
    creator: "GDELTADEV1111111111111111111111111111111111111111111111",
    state: "active",
    total_stake: 10000,
    created_at: "2026-01-01T00:00:00Z", // older
  },
  {
    pool_id: 404,
    name: "Delta Prediction Protocol 2",
    description: "Shared description text for delta testing.",
    tags: ["delta"],
    creator: "GDELTADEV1111111111111111111111111111111111111111111111",
    state: "active",
    total_stake: 10000,
    created_at: "2026-02-01T00:00:00Z", // newer -> wins tie-break
  },

  // Pair 3: Tied on score, tier, stake, and creation date, resolved by canonical pool_id
  {
    pool_id: 405,
    name: "Echo Protocol Market",
    description: "Identical parameters echo test.",
    tags: ["echo"],
    creator: "GECHODEV111111111111111111111111111111111111111111111111",
    state: "active",
    total_stake: 10000,
    created_at: "2026-01-01T00:00:00Z",
  },
  {
    pool_id: 406,
    name: "Echo Protocol Market",
    description: "Identical parameters echo test.",
    tags: ["echo"],
    creator: "GECHODEV111111111111111111111111111111111111111111111111",
    state: "active",
    total_stake: 10000,
    created_at: "2026-01-01T00:00:00Z",
  },
];
