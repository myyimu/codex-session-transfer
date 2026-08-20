export function normalizeDisplayPath(path) {
  if (typeof path !== "string") return "";
  if (path.startsWith("\\\\?\\UNC\\")) return `\\\\${path.slice(8)}`;
  if (path.startsWith("\\\\?\\")) return path.slice(4);
  return path;
}

export function isCodexWorktreePath(path) {
  return normalizeDisplayPath(path)
    .split(/[\\/]+/)
    .some((part, index, parts) => part.toLowerCase() === ".codex" && parts[index + 1]?.toLowerCase() === "worktrees");
}

export function prepareProjectMappings(mappings) {
  return (mappings || []).map((mapping) => {
    const candidates = (mapping.candidates || [])
      .map(normalizeDisplayPath)
      .filter((candidate) => !isCodexWorktreePath(candidate));
    const suggestedPath = normalizeDisplayPath(mapping.suggestedPath || "");
    const safeSuggestedPath = isCodexWorktreePath(suggestedPath) ? "" : suggestedPath;
    const keepUnbound = !safeSuggestedPath && candidates.length === 0;
    return {
      ...mapping,
      sourcePath: normalizeDisplayPath(mapping.sourcePath || ""),
      candidates,
      suggestedPath: safeSuggestedPath,
      targetCwd: safeSuggestedPath,
      keepUnbound,
    };
  });
}

export function projectMappingStatus(mapping) {
  const candidates = mapping.candidates || [];
  if (mapping.keepUnbound) return "本机没有对应目录，将保持未绑定";
  if (!mapping.targetCwd) {
    if (candidates.length > 1) return "发现多个同名目录，请选择";
    if (candidates.length === 1) return "已取消自动选择，请重新选择或浏览目录";
    return "请选择保持未绑定，或浏览本机目录";
  }
  if (mapping.status === "exact") return "路径完全一致，已自动预选";
  if (mapping.status === "suggested") return "已预选唯一同名目录，请确认";
  return "已手动选择本机目录";
}

export function repairableTaskIds(report) {
  return report?.tasks
    ?.filter((task) => task.safeActions?.includes("reregister") && !task.requiresManualReview)
    .map((task) => task.id) || [];
}

export function activeProjectMappings(mappings, tasks, restoreExisting) {
  const activeIds = new Set(
    (tasks || [])
      .filter((task) => !task.conflict || restoreExisting)
      .map((task) => task.id),
  );
  return (mappings || []).flatMap((mapping) => {
    const mappingTaskIds = mapping.taskIds?.length
      ? mapping.taskIds
      : (tasks || [])
        .filter((task) => (task.projectKey || task.cwd) === mapping.sourceKey)
        .map((task) => task.id);
    const activeTaskCount = mappingTaskIds.filter((id) => activeIds.has(id)).length;
    return activeTaskCount ? [{ ...mapping, activeTaskCount }] : [];
  });
}
