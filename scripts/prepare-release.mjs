#!/usr/bin/env node
// Local release preparation for Welkin.
//
// Reads the commits since the previous GitHub release, asks DeepSeek to turn
// them into a user-facing changelog, lets the maintainer review/edit it, then
// writes `changelog.md`, bumps the version and creates the local release tag.
//
// The DeepSeek key never leaves this machine: it is read from an argument, an
// environment variable, or a local file, and is never written to the repo.
//
// Usage: node scripts/prepare-release.mjs [tag] [options]
//   tag                    e.g. v1.0.1 (prompted when omitted)
//   --dry-run              preview only; do not write files / commit / tag
//   --no-commit            write changelog.md + bump version, but do not commit/tag
//   --yes                  accept the AI changelog without the review prompt
//   --prev <tag>           override the previous release tag
//   --api-key <key>        DeepSeek API key
//   --api-key-file <path>  file containing the DeepSeek API key
//   --model <name>         DeepSeek model (default: deepseek-chat)
//   --help

import { execFileSync } from "node:child_process";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { tmpdir } from "node:os";
import readline from "node:readline/promises";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const CHANGELOG_FILE = join(ROOT, "changelog.md");
const PACKAGE_FILE = join(ROOT, "package.json");
const DEFAULT_KEY_FILE = "D:\\Code\\YourDeepSeekAPIKey.txt";
const API_URL = "https://api.deepseek.com/chat/completions";

const SYSTEM_PROMPT = `你是 Welkin 桌面音乐播放器的发布说明撰写助手。根据 git 提交记录，撰写面向用户的更新日志。

严格遵守输出格式：
1. 只输出正文，不要标题、版本号、前言、解释或代码围栏。
2. 第一段为：
摘要
<用 1-3 句话概括本次更新的主要亮点>
3. 之后可包含以下段落，按此顺序，只保留非空段：
新增
- ...
优化
- ...
修复
- ...
其他
- ...
4. 每条一行，以 "- " 开头；面向用户、去重、合并同类改动；不要照抄提交信息。
5. 按语义归类：feat→新增；perf/refactor→优化；fix→修复；其余（chore/docs/build/ci/style/test 等）→其他；没有实质用户价值的内容可省略。
6. 使用简体中文。
7. 不编造提交记录中没有的功能。`;

function parseArgs(argv) {
  const options = { positional: [], model: "deepseek-chat" };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--help" || arg === "-h") options.help = true;
    else if (arg === "--dry-run") options.dryRun = true;
    else if (arg === "--no-commit") options.noCommit = true;
    else if (arg === "--yes" || arg === "-y") options.yes = true;
    else if (arg === "--push") options.push = true;
    else if (arg === "--no-push") options.push = false;
    else if (arg === "--prev") options.prev = argv[++i];
    else if (arg === "--api-key") options.apiKey = argv[++i];
    else if (arg === "--api-key-file") options.apiKeyFile = argv[++i];
    else if (arg === "--model") options.model = argv[++i];
    else if (arg.startsWith("--")) fail(`未知参数: ${arg}`);
    else options.positional.push(arg);
  }
  return options;
}

function usage() {
  console.log(`用法: node scripts/prepare-release.mjs [tag] [选项]

  tag                    例如 v1.0.1（省略则交互输入）
  --dry-run              只预览，不写文件/不提交/不打 tag
  --no-commit            写 changelog.md 并升版本，但不提交、不打 tag
  --yes                  跳过审查，直接采用 AI 结果
  --push                 提交/打 tag 后自动推送（默认交互询问）
  --no-push              提交/打 tag 后不推送
  --prev <tag>           手动指定上一个 release tag
  --api-key <key>        DeepSeek API Key
  --api-key-file <path>  从文件读取 DeepSeek API Key
  --model <name>         DeepSeek 模型（默认 deepseek-chat）
  --help`);
}

function fail(message) {
  console.error(`\n[错误] ${message}`);
  process.exit(1);
}

/** Run a command in the repo root and return trimmed stdout. */
function run(command, args, { allowFailure = false } = {}) {
  try {
    return execFileSync(command, args, { cwd: ROOT, encoding: "utf8" }).trim();
  } catch (error) {
    if (allowFailure) return "";
    const detail = (error.stderr || error.stdout || error.message || "").toString().trim();
    fail(`${command} ${args.join(" ")} 执行失败：${detail}`);
  }
}

function resolveApiKey(options) {
  if (options.apiKey?.trim()) return options.apiKey.trim();
  if (process.env.DEEPSEEK_API_KEY?.trim()) return process.env.DEEPSEEK_API_KEY.trim();
  const file = options.apiKeyFile || process.env.DEEPSEEK_API_KEY_FILE || DEFAULT_KEY_FILE;
  if (existsSync(file)) {
    const key = readFileSync(file, "utf8").trim();
    if (key) return key;
  }
  fail("未找到 DeepSeek API Key：请用 --api-key/--api-key-file 传入，或设置 DEEPSEEK_API_KEY。");
}

/** Ask DeepSeek and return the changelog text. */
async function generate(apiKey, model, messages) {
  const response = await fetch(API_URL, {
    method: "POST",
    headers: { "Content-Type": "application/json", Authorization: `Bearer ${apiKey}` },
    body: JSON.stringify({ model, messages, temperature: 0.3, stream: false }),
  });
  if (!response.ok) {
    const body = await response.text().catch(() => "");
    fail(`DeepSeek 请求失败（HTTP ${response.status}）：${body}`);
  }
  const data = await response.json();
  const content = data?.choices?.[0]?.message?.content?.trim();
  if (!content) fail("DeepSeek 返回了空结果。");
  return content;
}

/** Open $EDITOR (or notepad) on the given file and return its new contents. */
function editInEditor(file) {
  const editor = process.env.VISUAL || process.env.EDITOR || (process.platform === "win32" ? "notepad" : "nano");
  console.log(`\n正在打开编辑器：${editor}`);
  try {
    execFileSync(editor, [file], { stdio: "inherit" });
  } catch {
    // A non-zero exit from the editor is not fatal; fall back to whatever is saved.
  }
  return readFileSync(file, "utf8").trim();
}

function box(title, text) {
  const line = "─".repeat(60);
  console.log(`\n${line}\n${title}\n${line}\n${text}\n${line}`);
}

async function main() {
  const options = parseArgs(process.argv.slice(2));
  if (options.help) {
    usage();
    return;
  }

  const rl = readline.createInterface({ input: process.stdin, output: process.stdout });

  try {
    // 1. Resolve and validate the tag.
    let tag = options.positional[0];
    if (!tag) tag = (await rl.question("请输入本次发布的 tag（如 v1.0.1）：")).trim();
    if (!/^v\d+\.\d+\.\d+$/.test(tag)) fail(`tag 格式不合法：${tag}（应形如 v1.0.1）`);
    const version = tag.slice(1);

    // 2. Preflight.
    if (run("git", ["status", "--porcelain"])) {
      fail("工作区有未提交的改动，请先提交或暂存后再运行。");
    }
    if (run("git", ["rev-parse", "-q", "--verify", `refs/tags/${tag}`], { allowFailure: true })) {
      fail(`tag ${tag} 已存在。`);
    }
    const branch = run("git", ["rev-parse", "--abbrev-ref", "HEAD"]);

    // 3. Previous release -> commit range.
    const previous =
      options.prev ||
      run("gh", ["release", "list", "--exclude-drafts", "--exclude-pre-releases", "--limit", "1", "--json", "tagName", "--jq", ".[0].tagName"], {
        allowFailure: true,
      });
    const range = previous ? `${previous}..HEAD` : "HEAD";
    console.log(`\n上一版本：${previous || "（无，首次发布）"}`);
    console.log(`提交范围：${range}`);

    const raw = run("git", ["log", "--no-merges", "--format=%h%x1f%s%x1f%b%x1e", range], { allowFailure: true });
    const commits = raw
      .split("\x1e")
      .map((record) => record.trim())
      .filter(Boolean)
      .map((record) => {
        const [hash, subject, body = ""] = record.split("\x1f");
        return body.trim() ? `${hash}\t${subject}\n${body.trim()}` : `${hash}\t${subject}`;
      });
    if (commits.length === 0) fail(`自 ${previous || "开始"} 以来没有提交，无需发布。`);
    console.log(`提交数量：${commits.length}`);

    // 4. Ask DeepSeek.
    const apiKey = resolveApiKey(options);
    const userPrompt = `上一版本：${previous || "无（首次发布）"}\n新版本：${tag}\n\n提交记录（hash\t标题，缩进行为正文）：\n${commits.join("\n")}`;
    const messages = [
      { role: "system", content: SYSTEM_PROMPT },
      { role: "user", content: userPrompt },
    ];

    console.log("\n正在请求 DeepSeek 生成更新日志…");
    let changelog = await generate(apiKey, options.model, messages);

    // 5. Review loop (the maintainer owns the final wording).
    if (!options.yes) {
      for (;;) {
        box("AI 生成的更新日志（待审查）", changelog);
        const action = (await rl.question("\n[a] 接受  [e] 编辑  [r] 重新生成  [q] 退出 > ")).trim().toLowerCase();
        if (action === "" || action === "a") break;
        if (action === "q") {
          console.log("已取消，未做任何改动。");
          return;
        }
        if (action === "e") {
          const tmp = join(tmpdir(), `welkin-changelog-${Date.now()}.md`);
          writeFileSync(tmp, `${changelog}\n`);
          changelog = editInEditor(tmp).trim() || changelog;
          continue;
        }
        if (action === "r") {
          const feedback = (await rl.question("补充要求（可留空直接重生成）：")).trim();
          messages.push({ role: "assistant", content: changelog });
          messages.push({ role: "user", content: feedback ? `请根据以下反馈重新生成：\n${feedback}` : "请重新生成，措辞可以更精炼。" });
          console.log("\n正在重新请求 DeepSeek…");
          changelog = await generate(apiKey, options.model, messages);
          continue;
        }
        console.log("请输入 a / e / r / q。");
      }
    }

    if (options.dryRun) {
      box("预览（--dry-run，未写入任何文件）", changelog);
      console.log("\n完成。");
      return;
    }

    // 6. Write changelog.md and bump the version.
    writeFileSync(CHANGELOG_FILE, `${changelog}\n`);
    console.log(`\n已写入 ${CHANGELOG_FILE}`);

    const pkg = JSON.parse(readFileSync(PACKAGE_FILE, "utf8"));
    if (pkg.version !== version) {
      pkg.version = version;
      writeFileSync(PACKAGE_FILE, `${JSON.stringify(pkg, null, 2)}\n`);
      console.log(`已更新 package.json 版本：${version}`);
    }

    if (options.noCommit) {
      console.log("\n已生成文件（--no-commit）。请手动提交并打 tag：");
      printPushHelp(tag, branch);
      return;
    }

    // 7. Commit and tag locally.
    run("git", ["add", "changelog.md", "package.json"]);
    run("git", ["commit", "-m", `chore(release): ${tag}`]);
    run("git", ["tag", "-a", tag, "-m", `Welkin ${tag}`]);
    console.log(`\n已提交并创建本地 tag ${tag}。`);
    await maybePush(tag, branch, rl, options);
  } finally {
    rl.close();
  }
}

function printPushHelp(tag, branch) {
  console.log("\n手动推送到远端以触发发布：");
  console.log(`  git push origin ${branch}`);
  console.log(`  git push origin ${tag}`);
}

/** Ask before pushing; `--push`/`--no-push` skip the prompt. */
async function maybePush(tag, branch, rl, options) {
  if (options.push === undefined) {
    const answer = (await rl.question(`\n是否推送到远端并触发发布？[y/N] `)).trim().toLowerCase();
    options.push = answer === "y" || answer === "yes";
  }
  if (!options.push) {
    printPushHelp(tag, branch);
    return;
  }
  console.log("\n正在推送…");
  run("git", ["push", "origin", branch]);
  run("git", ["push", "origin", tag]);
  console.log("\n已推送。CI 将开始构建并发布。");
}

main().catch((error) => fail(error?.stack || String(error)));
