/**
 * canonize: loads the CANON.md in pi's own folder, then the project's, and the
 * house files their `@` lines name, into pi's context, the way Claude reads
 * them through CLAUDE.md.
 * Written by `canon`; delete this file to stop it.
 */

import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

/** The house files `canon` wires into pi for every project. */
const AGENT_CANON = "__AGENT_CANON__";

function resolve(target: string, fromDir: string): string {
	if (target.startsWith("~/")) return path.join(os.homedir(), target.slice(2));
	return path.resolve(fromDir, target);
}

/** The house files one CANON.md names, each as a section. */
function sections(canon: string, seen: Set<string>): string[] {
	if (!fs.existsSync(canon)) return [];
	const out: string[] = [];
	for (const line of fs.readFileSync(canon, "utf8").split("\n")) {
		const m = line.trim().match(/^@(\S+)$/);
		if (!m) continue;
		const file = resolve(m[1]!, path.dirname(canon));
		if (seen.has(file) || !fs.existsSync(file) || !fs.statSync(file).isFile()) continue;
		seen.add(file);
		out.push(`# ${path.basename(file)}\n\n${fs.readFileSync(file, "utf8").trim()}`);
	}
	return out;
}

export default function (pi: ExtensionAPI) {
	pi.on("before_agent_start", (event) => {
		const seen = new Set<string>();
		const all = [
			...sections(AGENT_CANON, seen),
			...sections(path.join(process.cwd(), "CANON.md"), seen),
		];
		if (all.length === 0) return;

		return { systemPrompt: `${event.systemPrompt}\n\n${all.join("\n\n---\n\n")}\n` };
	});
}
