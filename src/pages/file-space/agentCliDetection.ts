export type AgentCliCheckResult = "passed" | "notConfigured" | "checkFailed" | "missing";
export type AgentCheckState = "idle" | "checking" | "passed" | "installed" | "missing" | "error";

export interface AgentCliStatusResponse {
  key: string;
  installed: boolean;
  reachable: boolean;
  version?: string | null;
  checkState?: AgentCliCheckResult;
}

const runtimeSuccessTtlMs = 5 * 60 * 1_000;
let recentRuntimeSuccess: { key: string; verifiedAt: number } | null = null;

export function recordAgentCliRuntimeSuccess(key: string, verifiedAt = Date.now()) {
  recentRuntimeSuccess = { key, verifiedAt };
}

export function wasAgentCliRecentlySuccessful(key: string, now = Date.now()) {
  return recentRuntimeSuccess?.key === key
    && now - recentRuntimeSuccess.verifiedAt >= 0
    && now - recentRuntimeSuccess.verifiedAt <= runtimeSuccessTtlMs;
}

export function clearAgentCliRuntimeSuccess() {
  recentRuntimeSuccess = null;
}

export function withAgentCheckResult<Key extends string, Value>(
  checks: Record<Key, Value>,
  key: Key,
  result: Value,
) {
  return { ...checks, [key]: result };
}

export function agentCheckState(result: AgentCliStatusResponse): AgentCheckState {
  switch (result.checkState) {
    case "passed":
      return "passed";
    case "notConfigured":
      return "installed";
    case "checkFailed":
      return "error";
    case "missing":
      return "missing";
    default:
      // Compatibility with status responses produced before checkState was added.
      return result.reachable ? "passed" : result.installed ? "installed" : "missing";
  }
}
