import { useCallback } from "react";
import { api, errorMessage, type NodeDetails, type SystemInfo } from "../../lib/api";
import { formatBytes, formatDate, plural } from "../../lib/format";
import { useApp } from "../../state/app";

/** How many of a folder's largest items the prompt lists. */
const LISTED_ITEMS = 8;

/**
 * Writes `%USERPROFILE%` in place of the profile folder. That keeps the
 * username out of the prompt, and assistants know the variable anyway.
 */
function displayPath(path: string, profile: string | undefined): string {
  if (!profile) return path;
  const next = path.charAt(profile.length);
  if (path.slice(0, profile.length).toLowerCase() === profile.toLowerCase() && (next === "" || next === "\\")) {
    return `%USERPROFILE%${path.slice(profile.length)}`;
  }
  return path;
}

/** A prompt asking an AI assistant what a scanned folder or file is and whether it's safe to delete. */
export function cleanupPrompt(details: NodeDetails, system: SystemInfo | null): string {
  const { node } = details;
  const kind = node.isDir ? "folder" : "file";
  const os = !system ? "Windows" : system.osBuild >= 22000 ? "Windows 11" : "Windows 10";
  const facts = [formatBytes(node.size)];
  if (node.isDir) facts.push(plural(node.files, "file"));
  else if (node.modified) facts.push(`last modified ${formatDate(node.modified)}`);

  const lines = [
    `I'm freeing up disk space on ${os} and found this ${kind} using a lot of it:`,
    "",
    displayPath(details.path, system?.userProfile),
    facts.join(", "),
  ];
  if (details.children.length > 0) {
    lines.push("", "Its largest contents:");
    for (const c of details.children) lines.push(`- ${c.name}${c.isDir ? "\\" : ""}: ${formatBytes(c.size)}`);
    if (details.hiddenCount > 0) lines.push(`- ${plural(details.hiddenCount, "more item")}: ${formatBytes(details.hiddenBytes)}`);
  }
  lines.push(
    "",
    `1. What is this ${kind}, and which app or part of Windows creates it?`,
    "2. Is it safe to delete? What would break, what gets rebuilt or downloaded again, and could I lose anything I can't get back, like settings, saves, or personal files?",
    "3. Is there a better way to clear it than deleting it by hand, such as a cleanup command or a setting in the app that owns it?",
    "4. Will it grow back, and can I cap its size or move it to another drive?",
    "",
    "Start with a one-line verdict: safe to delete, safe with caveats, or don't delete. If the path alone isn't enough to be sure, say so and tell me what to check.",
  );
  return lines.join("\n");
}

/** Returns a callback that copies the cleanup prompt for a scan item and confirms with a toast. */
export function useAskAi(): (id: number) => Promise<void> {
  const { system, toast } = useApp();
  return useCallback(
    async (id: number) => {
      try {
        const details = await api.scanNode(id, LISTED_ITEMS);
        await navigator.clipboard.writeText(cleanupPrompt(details, system));
        const name = details.path.replace(/\\+$/, "").split("\\").pop() || details.path;
        toast(`Copied a prompt about ${name}`, { detail: "Paste it into ChatGPT, Claude, or any AI assistant." });
      } catch (e) {
        toast("Couldn't copy the prompt", { detail: errorMessage(e), tone: "danger" });
      }
    },
    [system, toast],
  );
}
