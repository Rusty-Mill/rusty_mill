// Starts the `echo_agent` example (a rusty_agui AgentHandler on rusty_serve)
// and resolves with its URL. `AGUI_ECHO_BIN` names a prebuilt binary (CI);
// otherwise the example is built and run through cargo.

import { spawn, type ChildProcess } from "node:child_process";
import { resolve } from "node:path";

const WORKSPACE = resolve(import.meta.dirname, "../../../../..");

export interface EchoAgent {
  url: string;
  stop(): void;
}

export function startEchoAgent(): Promise<EchoAgent> {
  // Relative to where `npm test` runs, not to the workspace the agent runs in.
  const bin = process.env.AGUI_ECHO_BIN && resolve(process.cwd(), process.env.AGUI_ECHO_BIN);
  const child: ChildProcess = bin
    ? spawn(bin, [], { cwd: WORKSPACE, stdio: ["ignore", "pipe", "inherit"] })
    : spawn(
        "cargo",
        ["run", "-q", "-p", "rusty_agui", "--features", "serve", "--example", "echo_agent"],
        { cwd: WORKSPACE, stdio: ["ignore", "pipe", "inherit"] },
      );
  return new Promise((resolveAgent, reject) => {
    let buffer = "";
    child.stdout!.on("data", (chunk: Buffer) => {
      buffer += chunk.toString();
      const match = /LISTENING (http:\/\/\S+)/.exec(buffer);
      if (match) {
        resolveAgent({ url: match[1], stop: () => child.kill() });
      }
    });
    child.on("error", reject);
    child.on("exit", (code) => reject(new Error(`echo_agent exited early with ${code}`)));
  });
}
