import test from "node:test";
import assert from "node:assert/strict";
import {
  activeProjectMappings,
  isCodexWorktreePath,
  normalizeDisplayPath,
  prepareProjectMappings,
  projectMappingStatus,
  repairableTaskIds,
} from "../src/taskLogic.js";

test("Windows extended paths are displayed normally", () => {
  assert.equal(normalizeDisplayPath("\\\\?\\C:\\Users\\Legion\\work\\demo"), "C:\\Users\\Legion\\work\\demo");
  assert.equal(normalizeDisplayPath("\\\\?\\UNC\\server\\share\\demo"), "\\\\server\\share\\demo");
});

test("macOS and POSIX project paths remain unchanged", () => {
  const projectPath = "/Users/yimu/work/novel";
  assert.equal(normalizeDisplayPath(projectPath), projectPath);
  assert.equal(isCodexWorktreePath(projectPath), false);
  assert.equal(isCodexWorktreePath("/Users/yimu/.codex/worktrees/4329/novel"), true);
});

test("Codex temporary worktrees are not offered as project targets", () => {
  assert.equal(isCodexWorktreePath("C:\\Users\\Legion\\.codex\\worktrees\\4329\\novel"), true);
  const [mapping] = prepareProjectMappings([{
    sourcePath: "\\\\?\\D:\\myword\\小说\\novel",
    candidates: ["\\\\?\\C:\\Users\\Legion\\.codex\\worktrees\\4329\\novel", "\\\\?\\D:\\myword\\小说\\novel"],
    suggestedPath: "\\\\?\\C:\\Users\\Legion\\.codex\\worktrees\\4329\\novel",
  }]);
  assert.equal(mapping.sourcePath, "D:\\myword\\小说\\novel");
  assert.deepEqual(mapping.candidates, ["D:\\myword\\小说\\novel"]);
  assert.equal(mapping.targetCwd, "");
});

test("unique project suggestion is preselected", () => {
  const [mapping] = prepareProjectMappings([{
    sourceKey: "old-demo",
    candidates: ["C:\\work\\demo"],
    suggestedPath: "C:\\work\\demo",
    status: "suggested",
  }]);
  assert.equal(mapping.targetCwd, "C:\\work\\demo");
  assert.equal(projectMappingStatus(mapping), "已预选唯一同名目录，请确认");
});

test("missing project suggestion defaults to an explicit unbound mapping", () => {
  const [mapping] = prepareProjectMappings([{
    sourceKey: "missing-project",
    sourcePath: "C:\\Users\\Old\\work\\missing-project",
    candidates: [],
    suggestedPath: "",
    status: "missing",
  }]);

  assert.equal(mapping.targetCwd, "");
  assert.equal(mapping.keepUnbound, true);
  assert.equal(projectMappingStatus(mapping), "本机没有对应目录，将保持未绑定");
});

test("clearing an automatic suggestion no longer reports it as selected", () => {
  const mapping = {
    candidates: ["C:\\work\\demo"],
    targetCwd: "",
    status: "exact",
  };
  assert.equal(projectMappingStatus(mapping), "已取消自动选择，请重新选择或浏览目录");
});

test("skipped conflicts do not require a project mapping", () => {
  const mappings = [
    { sourceKey: "new-project", taskIds: ["new-task"] },
    { sourceKey: "duplicate-project", taskIds: ["duplicate-task"] },
  ];
  const tasks = [
    { id: "new-task", conflict: false },
    { id: "duplicate-task", conflict: true },
  ];
  assert.deepEqual(
    activeProjectMappings(mappings, tasks, false).map((mapping) => mapping.sourceKey),
    ["new-project"],
  );
  assert.deepEqual(
    activeProjectMappings(mappings, tasks, true).map((mapping) => mapping.sourceKey),
    ["new-project", "duplicate-project"],
  );
});

test("only safe re-registration items are repairable", () => {
  const report = {
    tasks: [
      { id: "safe", safeActions: ["reregister"], requiresManualReview: false },
      { id: "manual", safeActions: ["reregister"], requiresManualReview: true },
      { id: "healthy", safeActions: [], requiresManualReview: false },
    ],
  };
  assert.deepEqual(repairableTaskIds(report), ["safe"]);
});
