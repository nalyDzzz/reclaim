import { AppWindow, Archive, Code, Database, Disc3, File, FileText, Film, Gamepad2, Image, Music, ScrollText } from "lucide-react";
import type { FileCategory } from "../../lib/api";

export const CATEGORIES: Record<FileCategory, { label: string; icon: React.ReactNode }> = {
  video: { label: "Videos", icon: <Film /> },
  images: { label: "Pictures", icon: <Image /> },
  audio: { label: "Music and audio", icon: <Music /> },
  archives: { label: "Archives", icon: <Archive /> },
  diskImages: { label: "Disk images and VMs", icon: <Disc3 /> },
  programs: { label: "Programs and libraries", icon: <AppWindow /> },
  documents: { label: "Documents", icon: <FileText /> },
  gameData: { label: "Game data", icon: <Gamepad2 /> },
  code: { label: "Code and build output", icon: <Code /> },
  logsAndTemp: { label: "Logs, caches, and temp files", icon: <ScrollText /> },
  data: { label: "Databases and app data", icon: <Database /> },
  other: { label: "Other files", icon: <File /> },
};
