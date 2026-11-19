import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtempSync, rmSync, existsSync, statSync, writeFileSync, mkdirSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";
import {
  Address,
  BASE_FEE,
  Contract,
  Keypair,
  TransactionBuilder,
  nativeToScVal,
  rpc,
  scValToNative,
} from "@stellar/stellar-sdk";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const rpcUrl = "http://localhost:8000/rpc";
const friendbotUrl = "http://localhost:8000/friendbot";
const localPassphrase = "Standalone Network ; February 2017";
const network = "wallet-e2e-local";

// Supported browser/device/wallet profiles. Keep in sync with docs/BROWSER_SUPPORT.md.
// Each profile is exercised by the critical wallet smoke flows: connect, simulate,
// sign, and recover. A regression in any critical flow fails the profile.
const criticalFlows = ["connect", "simulate", "sign", "recover", "claim", "refund"];
const browserMatrix = [
  { id: "chrome-desktop", browser: "chrome", device: "desktop", wallet: "freighter" },
  { id: "chrome-desktop-rabet", browser: "chrome", device: "desktop", wallet: "rabet" },
  { id: "firefox-desktop", browser: "firefox", device: "desktop", wallet: "freighter" },
  { id: "safari-desktop", browser: "safari", device: "desktop", wallet: "freighter" },
  { id: "edge-desktop", browser: "edge", device: "desktop", wallet: "freighter" },
  { id: "safari-mobile", browser: "safari", device: "mobile", wallet: "freighter" },
  { id: "chrome-mobile", browser: "chrome", device: "mobile", wallet: "freighter" },
];
const selectedProfile = process.env.WALLET_PROFILE || "";
const activeProfiles = selectedProfile
  ? browserMatrix.filter((profile) => profile.id === selectedProfile)
  : browserMatrix;
if (selectedProfile && activeProfiles.length === 0) {
  throw new Error(`Unknown WALLET_PROFILE "${selectedProfile}". Known profiles: ${browserMatrix.map((p) => p.id).join(", ")}`);
}

function publishProfileResult(profile, flows, error, transactions = []) {
  const dir = process.env.WALLET_RESULTS_DIR;
  if (!dir) return;
  mkdirSync(dir, { recursive: true });
  const payload = {
    profile: profile.id,
    browser: profile.browser,
    device: profile.device,
    wallet: profile.wallet,
    flows,
    status: error ? "failed" : "passed",
    error: error ? String(error.message || error) : null,
    // Every submitted transaction's hash + ledger, in submission order — on
    // a failure this is how you find the on-chain state the run left behind
    // without re-running it under a debugger.
    transactions,
  };
  writeFileSync(join(dir, `${profile.id}.json`), `${JSON.stringify(payload, null, 2)}\n`);
  if (error) {
    console.error(`[${profile.id}] FAILED — submitted transactions: ${JSON.stringify(transactions)}`);
  }
}

function command(program, args, cwd = root, quiet = false) {
  try {
    return execFileSync(program, args, {
      cwd,
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
      timeout: 600_000,
    }).trim();
  } catch (error) {
    const detail = quiet ? "" : `: ${error.stderr?.toString().trim() || error.message}`;
    throw new Error(`${program} ${args[0]} failed${detail}`);
  }
}

async function rpcProbe(method) {
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), 5_000);
  try {
    const response = await fetch(rpcUrl, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ jsonrpc: "2.0", id: 1, method }),
      signal: controller.signal,
    });
    if (!response.ok) throw new Error(`RPC returned HTTP ${response.status}`);
    const payload = await response.json();
    if (payload.error) throw new Error(`RPC ${method} failed`);
    return payload.result;
  } finally {
    clearTimeout(timeout);
  }
}

async function ready() {
  const deadline = Date.now() + 240_000;
  while (Date.now() < deadline) {
    try {
      const health = await rpcProbe("getHealth");
      const currentNetwork = await rpcProbe("getNetwork");
      if (health.status === "healthy") {
        assert.equal(currentNetwork.passphrase, localPassphrase, "RPC is not the local network");
        return;
      }
    } catch (error) {
      if (error?.code === "ERR_ASSERTION") throw error;
    }
    await new Promise((resolve) => setTimeout(resolve, 1_000));
  }
  throw new Error(`Local Soroban RPC did not become healthy at ${rpcUrl} within 240 seconds`);
}

function wasm(name) {
  const path = join(root, "contract", "target", "wasm32v1-none", "release", `${name}.wasm`);
  if (existsSync(path)) return path;
  throw new Error(`Built ${name}.wasm was not found at ${path}`);
}

async function fixture(server, configDir) {
  const cli = (...args) => command("stellar", ["--config-dir", configDir, ...args]);
  cli("network", "add", network, "--rpc-url", rpcUrl, "--network-passphrase", localPassphrase);
  command("stellar", ["contract", "build"], join(root, "contract"));
  command("stellar", ["--config-dir", configDir, "keys", "generate", "admin", "--as-secret"], root, true);
  const admin = cli("keys", "address", "admin");
  await server.fundAddress(admin, friendbotUrl);
  const wallet = Keypair.random();
  await server.fundAddress(wallet.publicKey(), friendbotUrl);

  const base = ["--source", "admin", "--network", network];
  const deploy = (name) => {
    const path = wasm(name);
    console.log(`Deploying ${name} (${statSync(path).size} WASM bytes)`);
    return cli("contract", "deploy", "--wasm", path, ...base);
  };
  const access = deploy("access_control");
  const core = deploy("predifi_contract");
  const token = cli("contract", "id", "asset", "--asset", "native", "--network", network);
  try {
    await server.getContractInstance(token);
  } catch {
    cli("contract", "asset", "deploy", "--asset", "native", ...base);
  }
  const invoke = (id, method, ...args) =>
    cli("contract", "invoke", "--id", id, ...base, "--", method, ...args);

  invoke(access, "init", "--admin", admin);
  // Settlement (`resolve_pool`) is gated on Operator role (1), distinct from
  // the Admin role (0) `init` already grants — settlement/claim coverage
  // below resolves pools as `admin`, so it needs Operator too.
  invoke(access, "assign_role", "--admin_caller", admin, "--user", admin, "--role", "Operator");
  invoke(core, "init", "--access_control", access, "--treasury", admin,
    "--fee_bps", "100", "--resolution_delay", "0", "--min_pool_duration", "0",
    "--max_predictions_per_user", "0");
  invoke(core, "add_token_to_whitelist", "--admin", admin, "--token", token);
  const baseConfig = {
    start_time: 0,
    description: "Local wallet signing test",
    metadata_url: "",
    min_stake: "1",
    max_stake: "0",
    min_total_stake: "1",
    max_total_stake: "0",
    initial_liquidity: "0",
    required_resolutions: 1,
    private: false,
    whitelist_key: null,
    outcome_descriptions: ["Yes", "No"],
  };

  /**
   * Create a pool ending `secondsFromNow` after the current ledger close
   * time (so it can be deliberately aged into an eligible-for-resolution or
   * overdue-for-cancellation state later in the test).
   */
  const createPool = async (secondsFromNow, configOverrides = {}) => {
    const ledgerTime = Number((await server.getLatestLedger()).closeTime);
    assert.ok(Number.isSafeInteger(ledgerTime), "RPC returned an invalid ledger close time");
    const config = { ...baseConfig, ...configOverrides };
    const id = Number(JSON.parse(invoke(core, "create_pool", "--creator", admin,
      "--end_time", String(ledgerTime + secondsFromNow), "--token", token,
      "--options_count", "2", "--category", "Tech", "--config", JSON.stringify(config))));
    return { id, endTime: ledgerTime + secondsFromNow };
  };

  const settlementPool = await createPool(3_600);
  assert.equal(settlementPool.id, 0, "Fresh contract should start at pool 0");
  return { core, admin, token, wallet, invoke, createPool, poolId: settlementPool.id };
}

/** Poll the local ledger until its close time passes `targetTime`. */
async function waitForLedgerTime(server, targetTime, timeoutMs = 60_000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const closeTime = Number((await server.getLatestLedger()).closeTime);
    if (closeTime >= targetTime) return closeTime;
    await new Promise((resolve) => setTimeout(resolve, 1_000));
  }
  throw new Error(`Ledger did not reach close time ${targetTime} within ${timeoutMs} ms`);
}

async function transaction(server, passphrase, core, wallet, poolId, outcome) {
  const account = await server.getAccount(wallet.publicKey());
  return new TransactionBuilder(account, { fee: BASE_FEE, networkPassphrase: passphrase })
    .addOperation(new Contract(core).call("place_prediction",
      new Address(wallet.publicKey()).toScVal(),
      nativeToScVal(BigInt(poolId), { type: "u64" }),
      nativeToScVal(10_000_000n, { type: "i128" }),
      nativeToScVal(outcome, { type: "u32" }),
      nativeToScVal(null),
      nativeToScVal(null)))
    .setTimeout(60)
    .build();
}

async function stakes(server, passphrase, core, wallet, poolId) {
  const account = await server.getAccount(wallet.publicKey());
  const read = new TransactionBuilder(account, { fee: BASE_FEE, networkPassphrase: passphrase })
    .addOperation(new Contract(core).call("get_pool_outcome_stakes",
      nativeToScVal(BigInt(poolId), { type: "u64" })))
    .setTimeout(60)
    .build();
  const simulation = await server.simulateTransaction(read);
  assert.ok(simulation.result, `Stake query failed: ${simulation.error || "missing result"}`);
  return scValToNative(simulation.result.retval).map(BigInt);
}

async function confirm(server, hash, timeoutMs = 30_000) {
  const deadline = Date.now() + timeoutMs;
  while (true) {
    const result = await server.getTransaction(hash);
    if (result.status === "SUCCESS") return result;
    if (result.status === "FAILED") throw new Error(`Prediction ${hash} failed on ledger`);
    if (Date.now() >= deadline) break;
    await new Promise((resolve) => setTimeout(resolve, 500));
  }
  throw new Error(`Prediction ${hash} was not confirmed within ${timeoutMs} ms`);
}

for (const profile of activeProfiles) {
  test(`local Soroban wallet signing flow [${profile.id}]`, async (t) => {
    const flows = Object.fromEntries(criticalFlows.map((flow) => [flow, "pending"]));
    const configDir = mkdtempSync(join(tmpdir(), "predifi-wallet-e2e-"));
    const submittedTransactions = [];
    try {
      try {
        command("docker", ["info", "--format", "{{.ServerVersion}}"]);
      } catch {
        throw new Error("Docker Engine is unavailable. Start Docker, or run Soroban Wallet E2E in GitHub Actions on your fork.");
      }
      try {
        command("stellar", ["--version"]);
      } catch {
        throw new Error("Stellar CLI is unavailable. Install version 28.1.0 before running the local wallet E2E.");
      }
      command("docker", ["compose", "-f", "docker-compose.yml", "-f", "e2e/wallet/compose.yml", "up", "-d", "stellar"]);
      const server = new rpc.Server(rpcUrl, { allowHttp: true });
      await ready();
      console.log(`[${profile.id}] Local Soroban protocol ${(await server.getLatestLedger()).protocolVersion}`);
      const { core, admin, wallet, invoke, createPool, poolId } = await fixture(server, configDir);
      let submissions = 0;
      const submit = async (signed) => {
        submissions++;
        const sent = await server.sendTransaction(signed);
        assert.equal(sent.status, "PENDING", `RPC rejected transaction: ${sent.status}`);
        const confirmed = await confirm(server, sent.hash);
        assert.equal(confirmed.status, "SUCCESS");
        assert.ok(Number(confirmed.ledger) > 0, "Confirmed transaction has no ledger");
        submittedTransactions.push({ hash: sent.hash, ledger: confirmed.ledger });
        return { hash: sent.hash, ledger: confirmed.ledger };
      };
      const signAndSubmit = async (signer) => {
        const raw = await transaction(server, localPassphrase, core, wallet, poolId, 0);
        const prepared = await server.prepareTransaction(raw);
        const signed = await signer(prepared);
        return submit(signed);
      };
      const before = await stakes(server, localPassphrase, core, wallet, poolId);
      assert.deepEqual(before, [0n, 0n]);

      await t.test("connect: wallet rejection prevents submission and state change", async () => {
        const rejected = Object.assign(new Error("User rejected signing"), { code: "USER_REJECTED" });
        await assert.rejects(signAndSubmit(async () => { throw rejected; }),
          (error) => error.code === "USER_REJECTED");
        assert.equal(submissions, 0);
        assert.deepEqual(await stakes(server, localPassphrase, core, wallet, poolId), before);
        flows.connect = "passed";
        console.log(`[${profile.id}] Wallet rejection: PASS (zero submissions)`);
      });

      await t.test("sign: signed prediction is confirmed and recorded", async () => {
        const { hash, ledger } = await signAndSubmit(async (prepared) => {
          prepared.sign(wallet);
          return prepared;
        });
        assert.match(hash, /^[a-f0-9]{64}$/i);
        assert.equal(submissions, 1);
        assert.deepEqual(await stakes(server, localPassphrase, core, wallet, poolId), [10_000_000n, 0n]);
        flows.sign = "passed";
        console.log(`[${profile.id}] Confirmed prediction: PASS (hash ${hash}, ledger ${ledger}, outcome 0 stake 10000000)`);
      });

      await t.test("simulate: invalid outcome returns contract error without changing stakes", async () => {
        const raw = await transaction(server, localPassphrase, core, wallet, poolId, 2);
        const simulation = await server.simulateTransaction(raw);
        assert.match(simulation.error || "", /#25\b|InvalidOutcome|Contract.*25/i);
        assert.equal(submissions, 1);
        assert.deepEqual(await stakes(server, localPassphrase, core, wallet, poolId), [10_000_000n, 0n]);
        flows.simulate = "passed";
        console.log(`[${profile.id}] Contract InvalidOutcome #25: PASS (unchanged stakes)`);
      });

      await t.test("recover: confirmation timeout has a bounded error", async () => {
        let checks = 0;
        const neverConfirmed = { getTransaction: async () => {
          checks++;
          return { status: "NOT_FOUND" };
        } };
        await assert.rejects(confirm(neverConfirmed, "0".repeat(64), 0), /was not confirmed within 0 ms/);
        assert.equal(checks, 1);
        assert.equal(submissions, 1);
        flows.recover = "passed";
        console.log(`[${profile.id}] Confirmation timeout: PASS (injected RPC response)`);
      });

      await t.test("claim: winnings are paid out once the pool resolves in the wallet's favor", async () => {
        const pool = await createPool(5);
        const raw = await transaction(server, localPassphrase, core, wallet, pool.id, 0);
        const prepared = await server.prepareTransaction(raw);
        prepared.sign(wallet);
        await submit(prepared);
        assert.deepEqual(await stakes(server, localPassphrase, core, wallet, pool.id), [10_000_000n, 0n]);

        // Settlement is only valid once the pool's end_time has actually
        // passed on-chain — wait for the local ledger to get there instead of
        // resolving early (which the contract would reject).
        await waitForLedgerTime(server, pool.endTime);
        invoke(core, "resolve_pool", "--operator", admin, "--pool_id", String(pool.id), "--outcome", "0");

        const account = await server.getAccount(wallet.publicKey());
        const claim = new TransactionBuilder(account, { fee: BASE_FEE, networkPassphrase: localPassphrase })
          .addOperation(new Contract(core).call("claim_winnings",
            new Address(wallet.publicKey()).toScVal(),
            nativeToScVal(BigInt(pool.id), { type: "u64" })))
          .setTimeout(60)
          .build();
        const preparedClaim = await server.prepareTransaction(claim);
        preparedClaim.sign(wallet);
        const { hash: claimHash, ledger: claimLedger } = await submit(preparedClaim);
        assert.match(claimHash, /^[a-f0-9]{64}$/i);

        // A second claim must be rejected — the payout already happened.
        const accountAfter = await server.getAccount(wallet.publicKey());
        const replay = new TransactionBuilder(accountAfter, { fee: BASE_FEE, networkPassphrase: localPassphrase })
          .addOperation(new Contract(core).call("claim_winnings",
            new Address(wallet.publicKey()).toScVal(),
            nativeToScVal(BigInt(pool.id), { type: "u64" })))
          .setTimeout(60)
          .build();
        const replaySimulation = await server.simulateTransaction(replay);
        assert.match(replaySimulation.error || "", /AlreadyClaimed|#2[0-9]\b/i);

        flows.claim = "passed";
        console.log(`[${profile.id}] Claimed winnings: PASS (pool ${pool.id}, hash ${claimHash}, ledger ${claimLedger})`);
      });

      await t.test("refund: stake is returned in full once the pool is canceled", async () => {
        const pool = await createPool(3_600);
        const raw = await transaction(server, localPassphrase, core, wallet, pool.id, 1);
        const prepared = await server.prepareTransaction(raw);
        prepared.sign(wallet);
        await submit(prepared);
        assert.deepEqual(await stakes(server, localPassphrase, core, wallet, pool.id), [0n, 10_000_000n]);

        invoke(core, "cancel_pool", "--operator", admin, "--pool_id", String(pool.id), "--reason", "E2E refund coverage");

        const account = await server.getAccount(wallet.publicKey());
        const refund = new TransactionBuilder(account, { fee: BASE_FEE, networkPassphrase: localPassphrase })
          .addOperation(new Contract(core).call("claim_refund",
            new Address(wallet.publicKey()).toScVal(),
            nativeToScVal(BigInt(pool.id), { type: "u64" })))
          .setTimeout(60)
          .build();
        const preparedRefund = await server.prepareTransaction(refund);
        preparedRefund.sign(wallet);
        const { hash: refundHash, ledger: refundLedger } = await submit(preparedRefund);
        assert.match(refundHash, /^[a-f0-9]{64}$/i);

        flows.refund = "passed";
        console.log(`[${profile.id}] Claimed refund: PASS (pool ${pool.id}, hash ${refundHash}, ledger ${refundLedger})`);
      });

      publishProfileResult(profile, flows, null, submittedTransactions);
    } catch (error) {
      publishProfileResult(profile, flows, error, submittedTransactions);
      throw error;
    } finally {
      rmSync(configDir, { recursive: true, force: true });
    }
  });
}
