import test from "node:test";
import assert from "node:assert/strict";
import { rewriteSessionCwd } from "../electron/codex-store.mjs";

test("rewriteSessionCwd updates nested JSON strings without changing JSONL shape", () => {
  const input = [
    JSON.stringify({ type: "session_meta", payload: { cwd: "E:\\project", nested: ["E:\\project\\src"] } }),
    JSON.stringify({ type: "message", payload: { role: "user", content: [{ type: "text", text: "keep me" }] } }),
    "",
  ].join("\n");
  const output = rewriteSessionCwd(input, "E:\\project", "/Users/me/work/project");
  const lines = output.trim().split("\n").map(JSON.parse);
  assert.equal(lines[0].payload.cwd, "/Users/me/work/project");
  assert.equal(lines[0].payload.nested[0], "/Users/me/work/project\\src");
  assert.equal(lines[1].payload.content[0].text, "keep me");
});

test("rewriteSessionCwd is a no-op when no mapping is available", () => {
  const input = '{"cwd":"/tmp/project"}\n';
  assert.equal(rewriteSessionCwd(input, "/tmp/project", "/tmp/project"), input);
});

