import { useCallback, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { settingsApi } from "@/lib/api";
import { syncCurrentProvidersLiveSafe } from "@/utils/postChangeSync";

export type ImportStatus =
  | "idle"
  | "importing"
  | "success"
  | "partial-success"
  | "error";

export interface UseImportExportOptions {
  onImportSuccess?: () => void | Promise<void>;
}

export interface WebDavConfig {
  webdavUrl?: string;
  webdavUsername?: string;
  webdavPassword?: string;
  webdavRemoteDir?: string;
  webdavFileName?: string;
}

export const DEFAULT_WEBDAV_URL = "https://dav.jianguoyun.com/dav/";
export const DEFAULT_WEBDAV_REMOTE_DIR = "cc-switch/backups";

export function buildDefaultWebdavBackupFileName(date = new Date()): string {
  const stamp = `${date.getFullYear()}${String(date.getMonth() + 1).padStart(2, "0")}${String(date.getDate()).padStart(2, "0")}_${String(date.getHours()).padStart(2, "0")}${String(date.getMinutes()).padStart(2, "0")}${String(date.getSeconds()).padStart(2, "0")}`;
  return `cc-switch-backup-${stamp}.zip`;
}

export interface UseImportExportResult {
  selectedFile: string;
  status: ImportStatus;
  errorMessage: string | null;
  backupId: string | null;
  isImporting: boolean;
  isWebdavPending: boolean;
  selectImportFile: () => Promise<void>;
  clearSelection: () => void;
  importConfig: () => Promise<void>;
  exportConfig: () => Promise<void>;
  backupToWebdav: (config: WebDavConfig) => Promise<void>;
  restoreFromWebdav: (config: WebDavConfig) => Promise<void>;
  resetStatus: () => void;
}

export function useImportExport(
  options: UseImportExportOptions = {},
): UseImportExportResult {
  const { t } = useTranslation();
  const { onImportSuccess } = options;

  const [selectedFile, setSelectedFile] = useState("");
  const [status, setStatus] = useState<ImportStatus>("idle");
  const [errorMessage, setErrorMessage] = useState<string | null>(null);
  const [backupId, setBackupId] = useState<string | null>(null);
  const [isImporting, setIsImporting] = useState(false);
  const [isWebdavPending, setIsWebdavPending] = useState(false);

  const buildWebdavRequest = useCallback(
    (config: WebDavConfig, options?: { forBackup?: boolean }) => {
      const url = config.webdavUrl?.trim() || DEFAULT_WEBDAV_URL;
      const remoteDir =
        config.webdavRemoteDir?.trim() || DEFAULT_WEBDAV_REMOTE_DIR;
      const fileName =
        config.webdavFileName?.trim() ||
        (options?.forBackup ? buildDefaultWebdavBackupFileName() : undefined);

      return {
        url,
        username: config.webdavUsername?.trim() || undefined,
        password: config.webdavPassword || undefined,
        remoteDir,
        fileName,
      };
    },
    [],
  );

  const clearSelection = useCallback(() => {
    setSelectedFile("");
    setStatus("idle");
    setErrorMessage(null);
    setBackupId(null);
  }, []);

  const selectImportFile = useCallback(async () => {
    try {
      const filePath = await settingsApi.openFileDialog();
      if (filePath) {
        setSelectedFile(filePath);
        setStatus("idle");
        setErrorMessage(null);
      }
    } catch (error) {
      console.error("[useImportExport] Failed to open file dialog", error);
      toast.error(
        t("settings.selectFileFailed", {
          defaultValue: "选择文件失败",
        }),
      );
    }
  }, [t]);

  const importConfig = useCallback(async () => {
    if (!selectedFile) {
      toast.error(
        t("settings.selectFileFailed", {
          defaultValue: "请选择有效的 SQL 备份文件",
        }),
      );
      return;
    }

    if (isImporting) return;

    setIsImporting(true);
    setStatus("importing");
    setErrorMessage(null);

    try {
      const result = await settingsApi.importConfigFromFile(selectedFile);
      if (!result.success) {
        setStatus("error");
        const message =
          result.message ||
          t("settings.configCorrupted", {
            defaultValue: "SQL 文件已损坏或格式不正确",
          });
        setErrorMessage(message);
        toast.error(message);
        return;
      }

      setBackupId(result.backupId ?? null);
      // 导入成功后立即触发外部刷新（与 live 同步结果解耦）
      // - 避免 sync 失败时 UI 不刷新
      // - 避免依赖 setTimeout（组件卸载会取消）
      void onImportSuccess?.();

      const syncResult = await syncCurrentProvidersLiveSafe();
      if (syncResult.ok) {
        setStatus("success");
        toast.success(
          t("settings.importSuccess", {
            defaultValue: "配置导入成功",
          }),
          { closeButton: true },
        );
      } else {
        console.error(
          "[useImportExport] Failed to sync live config",
          syncResult.error,
        );
        setStatus("partial-success");
        toast.warning(
          t("settings.importPartialSuccess", {
            defaultValue:
              "配置已导入，但同步到当前供应商失败。请手动重新选择一次供应商。",
          }),
        );
      }
    } catch (error) {
      console.error("[useImportExport] Failed to import config", error);
      setStatus("error");
      const message =
        error instanceof Error ? error.message : String(error ?? "");
      setErrorMessage(message);
      toast.error(
        t("settings.importFailedError", {
          defaultValue: "导入配置失败: {{message}}",
          message,
        }),
      );
    } finally {
      setIsImporting(false);
    }
  }, [isImporting, onImportSuccess, selectedFile, t]);

  const exportConfig = useCallback(async () => {
    try {
      const now = new Date();
      const stamp = `${now.getFullYear()}${String(now.getMonth() + 1).padStart(2, "0")}${String(now.getDate()).padStart(2, "0")}_${String(now.getHours()).padStart(2, "0")}${String(now.getMinutes()).padStart(2, "0")}${String(now.getSeconds()).padStart(2, "0")}`;
      const defaultName = `cc-switch-export-${stamp}.sql`;
      const destination = await settingsApi.saveFileDialog(defaultName);
      if (!destination) {
        toast.error(
          t("settings.selectFileFailed", {
            defaultValue: "请选择 SQL 备份保存路径",
          }),
        );
        return;
      }

      const result = await settingsApi.exportConfigToFile(destination);
      if (result.success) {
        const displayPath = result.filePath ?? destination;
        toast.success(
          t("settings.configExported", {
            defaultValue: "配置已导出",
          }) + `\n${displayPath}`,
          { closeButton: true },
        );
      } else {
        toast.error(
          t("settings.exportFailed", {
            defaultValue: "导出配置失败",
          }) + (result.message ? `: ${result.message}` : ""),
        );
      }
    } catch (error) {
      console.error("[useImportExport] Failed to export config", error);
      toast.error(
        t("settings.exportFailedError", {
          defaultValue: "导出配置失败: {{message}}",
          message: error instanceof Error ? error.message : String(error ?? ""),
        }),
      );
    }
  }, [t]);

  const backupToWebdav = useCallback(
    async (config: WebDavConfig) => {
      if (isWebdavPending) return;

      setIsWebdavPending(true);
      try {
        const request = buildWebdavRequest(config, { forBackup: true });
        const result = await settingsApi.uploadConfigBackupToWebdav(request);
        if (!result.success) {
          throw new Error(
            result.message ||
              t("settings.webdavBackupFailedError", {
                defaultValue: "WebDAV 全量备份失败",
              }),
          );
        }

        toast.success(
          t("settings.webdavBackupSuccess", {
            defaultValue: "全量备份已上传到 WebDAV",
          }) + (result.remoteUrl ? `\n${result.remoteUrl}` : ""),
          { closeButton: true },
        );
      } catch (error) {
        console.error("[useImportExport] Failed to backup to WebDAV", error);
        toast.error(
          t("settings.webdavBackupFailedError", {
            defaultValue: "WebDAV 全量备份失败: {{message}}",
            message:
              error instanceof Error ? error.message : String(error ?? ""),
          }),
        );
      } finally {
        setIsWebdavPending(false);
      }
    },
    [buildWebdavRequest, isWebdavPending, t],
  );

  const restoreFromWebdav = useCallback(
    async (config: WebDavConfig) => {
      if (isImporting || isWebdavPending) return;

      setIsWebdavPending(true);
      setStatus("importing");
      setErrorMessage(null);

      try {
        const request = buildWebdavRequest(config);
        const result =
          await settingsApi.downloadConfigBackupFromWebdav(request);
        if (!result.success) {
          setStatus("error");
          const message =
            result.message ||
            t("settings.webdavRestoreFailedError", {
              defaultValue: "从 WebDAV 全量恢复失败",
            });
          setErrorMessage(message);
          toast.error(message);
          return;
        }

        setBackupId(result.backupId ?? null);
        void onImportSuccess?.();

        const syncResult = await syncCurrentProvidersLiveSafe();
        if (syncResult.ok) {
          setStatus("success");
          toast.success(
            t("settings.webdavRestoreSuccess", {
              defaultValue: "已从 WebDAV 完成全量恢复",
            }),
            { closeButton: true },
          );
        } else {
          console.error(
            "[useImportExport] Failed to sync live config after WebDAV restore",
            syncResult.error,
          );
          setStatus("partial-success");
          toast.warning(
            t("settings.importPartialSuccess", {
              defaultValue:
                "配置已导入，但同步到当前供应商失败。请手动重新选择一次供应商。",
            }),
          );
        }
      } catch (error) {
        console.error("[useImportExport] Failed to restore from WebDAV", error);
        setStatus("error");
        const message =
          error instanceof Error ? error.message : String(error ?? "");
        setErrorMessage(message);
        toast.error(
          t("settings.webdavRestoreFailedError", {
            defaultValue: "从 WebDAV 全量恢复失败: {{message}}",
            message,
          }),
        );
      } finally {
        setIsWebdavPending(false);
      }
    },
    [buildWebdavRequest, isImporting, isWebdavPending, onImportSuccess, t],
  );

  const resetStatus = useCallback(() => {
    setStatus("idle");
    setErrorMessage(null);
    setBackupId(null);
  }, []);

  return {
    selectedFile,
    status,
    errorMessage,
    backupId,
    isImporting,
    isWebdavPending,
    selectImportFile,
    clearSelection,
    importConfig,
    exportConfig,
    backupToWebdav,
    restoreFromWebdav,
    resetStatus,
  };
}
