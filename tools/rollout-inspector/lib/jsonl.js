const fs = require("node:fs");
const readline = require("node:readline");

function bytesOf(value) {
  return Buffer.byteLength(value, "utf8");
}

function safeJsonParse(line) {
  try {
    return { ok: true, value: JSON.parse(line) };
  } catch (error) {
    return {
      ok: false,
      error: error instanceof Error ? error.message : String(error),
    };
  }
}

async function* readJsonlLines(filePath) {
  const stream = fs.createReadStream(filePath, { encoding: "utf8" });
  const rl = readline.createInterface({
    input: stream,
    crlfDelay: Infinity,
  });

  let lineNo = 0;
  for await (const line of rl) {
    lineNo += 1;
    yield {
      lineNo,
      line,
      lineBytes: bytesOf(line) + 1,
    };
  }
}

module.exports = {
  bytesOf,
  readJsonlLines,
  safeJsonParse,
};

