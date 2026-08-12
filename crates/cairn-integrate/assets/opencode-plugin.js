// Cairn — persistent project memory for OpenCode.
//
// Cairn generates this file in its entirety and owns every byte of it. It is
// installed by dropping it into an OpenCode config directory's `plugin/`
// folder, which OpenCode auto-discovers; no mutation of `opencode.json` is
// involved (D32).
//
// Everything this plugin does is translate an OpenCode signal into one
// `cairn hook <event>` invocation. It makes no decision about meaning: the
// adapter in `cairn-integrate` does that, so the plugin and the Rust side
// cannot disagree.
//
// It never blocks OpenCode. Every invocation is fire-and-forget and every
// failure is swallowed — Cairn is never the reason a session breaks.

const { spawn } = require("node:child_process");

function send(event, payload) {
  try {
    const child = spawn("cairn", ["hook", event], {
      stdio: ["pipe", "ignore", "ignore"],
      detached: false,
    });
    child.on("error", () => {});
    child.stdin.on("error", () => {});
    child.stdin.end(JSON.stringify(payload));
  } catch {
    // A missing binary, a full pipe, anything at all: the session continues.
  }
}

export const CairnPlugin = async ({ project, directory }) => {
  const cwd = directory || (project && project.worktree) || process.cwd();

  return {
    // Session lifecycle arrives on the event bus.
    event: async ({ event }) => {
      if (!event || !event.type) return;
      const sessionID =
        (event.properties && (event.properties.sessionID || event.properties.id)) ||
        event.sessionID;
      if (!sessionID) return;

      switch (event.type) {
        case "session.created":
          send("session.created", { sessionID, cwd });
          break;
        case "session.idle":
          // Quiescence only. OpenCode signals no session end, and Cairn does
          // not invent one.
          send("session.idle", { sessionID, cwd });
          break;
        case "session.compacted":
          send("session.compacted", { sessionID, cwd });
          break;
        default:
          break;
      }
    },

    // Tool lifecycle arrives on plugin hooks.
    "tool.execute.after": async (input, output) => {
      const sessionID = input && (input.sessionID || input.sessionId);
      if (!sessionID) return;
      send("tool.execute.after", {
        sessionID,
        cwd,
        tool: input.tool,
        // Only the fields Cairn's allow-list retains are forwarded. The
        // output's own text and metadata are deliberately not sent.
        args: pickArgs(input.args),
        output: pickOutcome(output),
      });
    },

    // Experimental in OpenCode; where it is absent this simply never fires and
    // the capability is reported absent rather than assumed.
    "experimental.session.compacting": async (input) => {
      const sessionID = input && input.sessionID;
      if (!sessionID) return;
      send("experimental.session.compacting", { sessionID, cwd });
    },
  };
};

// The allow-listed input fields, and nothing else.
function pickArgs(args) {
  if (!args || typeof args !== "object") return {};
  const out = {};
  if (typeof args.filePath === "string") out.file_path = args.filePath;
  if (typeof args.file_path === "string") out.file_path = args.file_path;
  if (typeof args.path === "string") out.file_path = args.path;
  if (typeof args.command === "string") out.command = args.command;
  return out;
}

// The derived outcome only — never the output text.
function pickOutcome(output) {
  if (!output || typeof output !== "object") return {};
  const out = {};
  if (typeof output.exitCode === "number") out.exit_code = output.exitCode;
  if (typeof output.exit_code === "number") out.exit_code = output.exit_code;
  if (output.error) {
    out.error = {
      message:
        typeof output.error === "string"
          ? output.error
          : String((output.error && output.error.message) || "error"),
    };
  }
  if (output.failed === true) out.failed = true;
  return out;
}

export default CairnPlugin;
