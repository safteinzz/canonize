/**
 * canonize: loads the project's CANON.md, and the house files its `@` lines
 * name, into pi's context, the way Claude reads it through CLAUDE.md.
 * Written by `canon`; delete this file to stop it.
 */

import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

function resolve(target: string, fromDir: string): string {
	if (target.startsWith("~/")) return path.join(os.homedir(), target.slice(2));
	return path.resolve(fromDir, target);
}

export default function (pi: ExtensionAPI) {
	pi.on("before_agent_start", (event) => {
		const dir = process.cwd();
		const canon = path.join(dir, "CANON.md");
		if (!fs.existsSync(canon)) return;

		const sections: string[] = [];
		for (const line of fs.readFileSync(canon, "utf8").split("\n")) {
			const m = line.trim().match(/^@(\S+)$/);
			if (!m) continue;
			const file = resolve(m[1]!, dir);
			if (!fs.existsSync(file) || !fs.statSync(file).isFile()) continue;
			sections.push(`# ${path.basename(file)}\n\n${fs.readFileSync(file, "utf8").trim()}`);
		}
		if (sections.length === 0) return;

		return { systemPrompt: `${event.systemPrompt}\n\n${sections.join("\n\n---\n\n")}\n` };
	});
}
