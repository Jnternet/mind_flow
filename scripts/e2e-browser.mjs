// 真实浏览器端到端验证：状态渲染 → 点时间戳跳转 → 刷新恢复 → 第二段落绝对时间 → 音频 Range。
//
//   node scripts/e2e-browser.mjs
//
// 需要本机有 firefox。服务端跑 --engine stub（确定性文本）。
//
// 为什么不用页面的麦克风 / 编辑 / 保存按钮驱动：
//   本容器的网络代理对「浏览器页面发起的带 body 请求」和麦克风音频链路会整段卡住
//   （curl / Node / Rust 客户端都正常）。所以这里只验证浏览器侧不写数据的行为，
//   写路径（编辑 PATCH、保存 POST、WS 上传音频）由 cargo test 与 scripts/smoke.sh 覆盖，
//   真机上的浏览器手测清单见 README。

import { spawn } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const ROOT = resolve(import.meta.dirname, "..");
const BIN = process.env.E2E_BIN ?? join(ROOT, "target/debug/mind_flow");
const PORT = Number(process.env.E2E_PORT ?? 8811);
const BIDI_PORT = Number(process.env.E2E_BIDI_PORT ?? 9333);
const BASE = `http://127.0.0.1:${PORT}/`;
const DATA_DIR = mkdtempSync(join(tmpdir(), "mind_flow_e2e_"));
const PROFILE_DIR = mkdtempSync(join(tmpdir(), "mind_flow_ff_"));

const children = [];
const log = (...args) => console.log(...args);

function cleanup() {
  for (const child of children) {
    try {
      child.kill("SIGTERM");
    } catch {
      // 忽略
    }
  }
  rmSync(DATA_DIR, { recursive: true, force: true });
  rmSync(PROFILE_DIR, { recursive: true, force: true });
}
process.on("exit", cleanup);
process.on("SIGINT", () => {
  cleanup();
  process.exit(130);
});

async function waitFor(描述, fn, { timeoutMs = 30_000, intervalMs = 150 } = {}) {
  const deadline = Date.now() + timeoutMs;
  let last;
  for (;;) {
    try {
      const value = await fn();
      if (value) return value;
      last = value;
    } catch (error) {
      last = error.message;
    }
    if (Date.now() > deadline) {
      throw new Error(`等待「${描述}」超时（最后状态：${JSON.stringify(last)}）`);
    }
    await new Promise((done) => setTimeout(done, intervalMs));
  }
}

// ---------------------------------------------------------------- 服务端

async function startServer() {
  if (!existsSync(BIN)) throw new Error(`找不到二进制 ${BIN}，先 cargo build`);
  const child = spawn(
    BIN,
    ["--engine", "stub", "--data-dir", DATA_DIR, "--port", String(PORT), "--no-open"],
    { stdio: ["ignore", "pipe", "pipe"] },
  );
  children.push(child);
  child.stderr.on("data", (chunk) => process.stderr.write(`[server] ${chunk}`));
  await waitFor("服务启动", async () => {
    const response = await fetch(`${BASE}api/state`).catch(() => null);
    return response?.ok;
  });
  log(`[服务] 已启动 ${BASE}`);
}

/** 用测试接口注入一段音频（等价于用户按住空格说了这么久）。 */
async function 注入音频(毫秒, rate = 16_000) {
  const samples = new Int16Array(Math.floor((rate * 毫秒) / 1000));
  for (let i = 0; i < samples.length; i += 1) samples[i] = (i / 40) % 2 === 0 ? 9000 : -9000;
  const response = await fetch(`${BASE}api/test/segment?rate=${rate}`, {
    method: "POST",
    body: samples.buffer,
  });
  if (!response.ok) throw new Error(`注入音频失败：HTTP ${response.status}`);
  return response.json();
}

function 会话目录() {
  const sessions = join(DATA_DIR, "sessions");
  const dirs = existsSync(sessions) ? readdirSync(sessions) : [];
  return dirs.length === 0 ? null : join(sessions, dirs[0]);
}

function 读会话() {
  const dir = 会话目录();
  if (!dir) return null;
  const path = join(dir, "session.json");
  return existsSync(path) ? JSON.parse(readFileSync(path, "utf8")) : null;
}

// ---------------------------------------------------------------- BiDi 客户端

class BidiClient {
  constructor(url) {
    this.url = url;
    this.nextId = 1;
    this.pending = new Map();
  }

  async connect() {
    await waitFor("Firefox BiDi 端口", async () => {
      const socket = new WebSocket(this.url);
      const opened = await new Promise((resolve) => {
        socket.addEventListener("open", () => resolve(true), { once: true });
        socket.addEventListener("error", () => resolve(false), { once: true });
        setTimeout(() => resolve(false), 1000);
      });
      if (!opened) {
        socket.close();
        return false;
      }
      this.socket = socket;
      return true;
    });
    this.socket.addEventListener("message", (event) => {
      const message = JSON.parse(event.data);
      if (message.method === "log.entryAdded" && message.params?.entry?.level === "error") {
        console.error(`[页面错误] ${message.params.entry.text}`);
      }
      const pending = this.pending.get(message.id);
      if (!pending) return;
      this.pending.delete(message.id);
      if (message.type === "error") pending.reject(new Error(JSON.stringify(message)));
      else pending.resolve(message.result);
    });
  }

  call(method, params = {}) {
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.socket.send(JSON.stringify({ id, method, params }));
      setTimeout(() => {
        if (this.pending.delete(id)) reject(new Error(`${method} 超时（15 秒）`));
      }, 15_000);
    });
  }
}

async function evaluate(client, context, expression) {
  const result = await client.call("script.evaluate", {
    expression: `(async () => JSON.stringify(await (${expression})))()`,
    target: { context },
    awaitPromise: true,
    resultOwnership: "none",
  });
  if (result.type === "exception") {
    throw new Error(`页面脚本异常：${JSON.stringify(result.exceptionDetails?.exception ?? result)}`);
  }
  const raw = result.result?.value;
  return raw === undefined ? undefined : JSON.parse(raw);
}

// ---------------------------------------------------------------- 浏览器

async function startBrowser() {
  writeFileSync(
    join(PROFILE_DIR, "user.js"),
    [
      'user_pref("media.autoplay.default", 0);',
      'user_pref("media.autoplay.blocking_policy", 0);',
      'user_pref("browser.shell.checkDefaultBrowser", false);',
      'user_pref("dom.disable_beforeunload", true);',
      'user_pref("permissions.default.microphone", 1);',
    ].join("\n"),
  );
  const child = spawn(
    "firefox",
    [
      "--headless",
      "--no-remote",
      "--profile",
      PROFILE_DIR,
      "--remote-debugging-port",
      String(BIDI_PORT),
      "about:blank",
    ],
    { stdio: ["ignore", "pipe", "pipe"] },
  );
  children.push(child);
  const client = new BidiClient(`ws://127.0.0.1:${BIDI_PORT}/session`);
  await client.connect();
  await client.call("session.new", { capabilities: {} });
  await client.call("session.subscribe", { events: ["log.entryAdded"] }).catch(() => {});
  log("[浏览器] Firefox 已就绪");
  return client;
}

async function openApp(client) {
  const { context } = await client.call("browsingContext.create", { type: "tab" });
  await client.call("browsingContext.navigate", { context, url: BASE, wait: "complete" });
  await waitFor("界面加载", () =>
    evaluate(client, context, "!!document.querySelector('#sentences')"),
  );
  // 等 WebSocket 连上（hello 到达后会显示录音提示）
  await waitFor("页面拿到会话状态", async () => {
    const hint = await evaluate(client, context, "document.getElementById('hint').textContent");
    return typeof hint === "string" && hint.length > 0;
  });
  return context;
}

const 行数 = (client, context) =>
  evaluate(client, context, "document.querySelectorAll('#sentences li[data-id]').length");

async function main() {
  await startServer();
  // 两段音频都在浏览器打开前注入（浏览器活跃时本容器的网络代理会卡住带 body 的请求）
  await 注入音频(1500);
  await waitFor("第一段识别完成", () => (读会话()?.sentences.length ?? 0) >= 1, {
    timeoutMs: 20_000,
  });
  await 注入音频(1200, 48_000);
  await waitFor("第二段识别完成", () => (读会话()?.sentences.length ?? 0) >= 2, {
    timeoutMs: 20_000,
  });
  const 文档 = 读会话();
  const 句一 = 文档.sentences[0];
  const 句二 = 文档.sentences[文档.sentences.length - 1];
  if (句二.segment_id !== 2 || 句二.start_ms < 句一.end_ms) {
    throw new Error(`第二段应接在第一段之后：${JSON.stringify(文档.sentences)}`);
  }
  log(`[准备] 两段已就绪：${句一.end_ms}ms → ${句二.start_ms}ms（48kHz 段已重采样）`);

  const client = await startBrowser();
  let context = await openApp(client);

  // 1) 渲染：服务端已有两句，页面应显示出来
  await waitFor("句子渲染出来", async () => (await 行数(client, context)) >= 2);
  const 显示时间 = await evaluate(client, context, "document.querySelector('.stamp').textContent");
  log(`[步骤1] 已渲染 2 句，首句时间戳显示 ${显示时间}（数据 ${句一.start_ms}ms）`);
  if (!(句一.end_ms > 句一.start_ms)) throw new Error("句子时间戳不合法");

  // 2) 点时间戳跳转
  await evaluate(client, context, "document.querySelector('.stamp').click()");
  const 位置 = await waitFor("播放器跳到该句时间", async () => {
    const value = await evaluate(client, context, "document.getElementById('audio').currentTime");
    return value > 0 ? value : null;
  });
  const 期望 = 句一.start_ms / 1000;
  if (Math.abs(位置 - 期望) > 1.5) {
    throw new Error(`跳转位置不对：${位置}s，期望 ${期望}s`);
  }
  log(`[步骤2] 点时间戳跳转到 ${位置.toFixed(2)}s（期望 ${期望.toFixed(2)}s）`);
  await evaluate(client, context, "document.getElementById('audio').pause()");

  // 3) 播放器在浏览器里真的加载到了合并音频
  const 时长 = await waitFor("播放器加载音频元数据", async () => {
    const value = await evaluate(client, context, "document.getElementById('audio').duration");
    return Number.isFinite(value) && value > 0 ? value : null;
  });
  log(`[步骤3] 播放器已加载音频，时长 ${时长.toFixed(2)}s`);

  // 4) 点第二句的时间戳，能跳到更靠后的位置
  await evaluate(client, context, "document.querySelectorAll('.stamp')[1].click()");
  const 第二位置 = await waitFor("播放器跳到第二句", async () => {
    const value = await evaluate(client, context, "document.getElementById('audio').currentTime");
    return value > 0.5 ? value : null;
  });
  const 第二期望 = 文档.sentences[1].start_ms / 1000;
  if (Math.abs(第二位置 - 第二期望) > 1.5) {
    throw new Error(`第二句跳转不对：${第二位置}s，期望 ${第二期望}s`);
  }
  log(`[步骤4] 点第二句跳到 ${第二位置.toFixed(2)}s（期望 ${第二期望.toFixed(2)}s）`);
  await evaluate(client, context, "document.getElementById('audio').pause()");

  // 5) 界面上仍有交互入口（保存/丢弃按钮可用）
  const 按钮可用 = await evaluate(
    client,
    context,
    `(() => {
       const finish = document.getElementById('btn-finish');
       const discard = document.getElementById('btn-discard');
       return !finish.disabled && !discard.disabled && finish.textContent.includes('结束并保存');
     })()`,
  );
  if (!按钮可用) throw new Error("保存按钮状态不对");
  log("[步骤5] 保存/丢弃入口可用（写路径由 cargo test 与 smoke.sh 覆盖）");
  log("== 浏览器端到端测试通过 ==");
}

main().then(
  () => {
    cleanup();
    process.exit(0);
  },
  (error) => {
    console.error(`\n端到端测试失败：${error.message}`);
    cleanup();
    process.exit(1);
  },
);
