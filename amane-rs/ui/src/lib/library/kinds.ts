/**
 * 媒体库归类.
 *
 * 顺序固定: 参与处理的类别在前, 未识别与残留目录兜底, 防止任何一类被静默忽略.
 */
export interface ScanKind {
  value: string;
  label: string;
}

export const SCAN_KINDS: readonly ScanKind[] = [
  { value: "media", label: "媒体文件" },
  { value: "unwanted", label: "无效文件" },
  { value: "sidecar", label: "附属文件" },
  { value: "system", label: "系统产物" },
  { value: "leftover_dir", label: "残留目录" },
  { value: "unrecognized", label: "未识别" },
];

export function kindLabel(value: string): string {
  return SCAN_KINDS.find((kind) => kind.value === value)?.label ?? value;
}

/** 无效文件的命中原因; 其余取值是后端规则名, 原样展示. */
export function ruleLabel(value: string): string {
  switch (value) {
    case "blacklist":
      return "黑名单";
    case "undersized":
      return "低于大小阈值";
    default:
      return value;
  }
}
