#!/usr/bin/env node
const path = require("node:path");
const { analyzeRollout } = require("./lib/analyzer");

function usage() {
  console.log(`Usage:
  node tools/rollout-inspector/analyze-rollout.js <rollout.jsonl> [options]

Options:
  --top <n>            Number of top records to show (default: 20)
  --large-kb <n>       "Huge payload" threshold in KiB (default: 256)
  --json               Print JSON instead of text
`);
}

function parseArgs(argv) {
  const args = {
    top: 20,
    largeKb: 256,
    json: false,
    file: null,
  };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--help" || arg === "-h") {
      usage();
      process.exit(0);
    }
    if (arg === "--top") {
      args.top = Number.parseInt(argv[i + 1] ?? "", 10);
      i += 1;
      continue;
    }
    if (arg === "--large-kb") {
      args.largeKb = Number.parseInt(argv[i + 1] ?? "", 10);
      i += 1;
      continue;
    }
    if (arg === "--json") {
      args.json = true;
      continue;
    }
    if (!args.file) {
      args.file = arg;
    }
  }
  if (!args.file) {
    usage();
    process.exit(1);
  }
  if (!Number.isFinite(args.top) || args.top <= 0) {
    throw new Error(`invalid --top value: ${args.top}`);
  }
  if (!Number.isFinite(args.largeKb) || args.largeKb <= 0) {
    throw new Error(`invalid --large-kb value: ${args.largeKb}`);
  }
  return args;
}

function formatBytes(bytes) {
  const units = ["B", "KiB", "MiB", "GiB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toFixed(value >= 10 || unit === 0 ? 0 : 1)} ${units[unit]}`;
}

function printSection(title) {
  console.log(`\n== ${title} ==`);
}

function printList(rows, toLine) {
  if (rows.length === 0) {
    console.log("(none)");
    return;
  }
  for (const row of rows) {
    console.log(toLine(row));
  }
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  const filePath = path.resolve(args.file);
  const report = await analyzeRollout(filePath, {
    topN: args.top,
    largeThresholdBytes: args.largeKb * 1024,
  });

  if (args.json) {
    console.log(JSON.stringify(report, null, 2));
    return;
  }

  console.log(`File: ${report.file}`);
  console.log(
    `Totals: ${report.totals.lines} line(s), ${formatBytes(report.totals.bytes)} (${report.totals.bytes} bytes)`,
  );

  if (report.parseErrors.length > 0) {
    printSection("Parse Errors");
    printList(report.parseErrors, (e) => `line ${e.line}: ${e.error}`);
  }

  printSection("Largest Entries");
  printList(
    report.largestEntries,
    (entry) =>
      `line ${entry.line} ${formatBytes(entry.bytes)} ${entry.type}/${entry.payloadType} :: ${entry.preview}`,
  );

  printSection("Exact Duplicate Large Payloads");
  printList(
    report.exactDuplicateLargePayloads,
    (dup) =>
      `${dup.count}x ${formatBytes(dup.bytes)} each (${formatBytes(dup.bytes * dup.count)} total) lines=${dup.lines.join(",")} :: ${dup.preview}`,
  );

  printSection("Near-Duplicate Large Payloads");
  printList(
    report.nearDuplicateLargePayloads,
    (dup) =>
      `${dup.count}x ${formatBytes(dup.totalBytes)} total lines=${dup.lines.join(",")} :: ${dup.normalizedPreview}`,
  );

  printSection("Repeated Tool Calls With Large Output");
  printList(
    report.repetitiveToolCalls,
    (call) =>
      `${call.callName} ${call.count}x ${formatBytes(call.totalOutputBytes)} total max=${formatBytes(call.maxOutputBytes)} lines=${call.lines.join(",")} args=${call.argsPreview}`,
  );

  printSection("Prune Candidates");
  printList(
    report.pruneCandidates,
    (candidate) =>
      `line ${candidate.line} ${formatBytes(candidate.bytes)} ${candidate.reason} ${candidate.callName ? `[${candidate.callName}]` : ""} :: ${candidate.preview}`,
  );
}

main().catch((error) => {
  console.error(error instanceof Error ? error.message : String(error));
  process.exit(1);
});

