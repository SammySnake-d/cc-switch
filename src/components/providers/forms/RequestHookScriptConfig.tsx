import { useTranslation } from "react-i18next";
import { useMemo, useState } from "react";
import { ChevronDown, ChevronRight, Code2, Play, Wand2 } from "lucide-react";
import { toast } from "sonner";
import JsonEditor from "@/components/JsonEditor";
import { Switch } from "@/components/ui/switch";
import { Button } from "@/components/ui/button";
import { providersApi } from "@/lib/api/providers";

/** 请求/响应重写脚本（onRequest/onResponse）配置 */
export interface RequestHookScriptConfig {
  enabled: boolean;
  language: "javascript";
  code: string;
  timeoutMs?: number;
}

export const defaultRequestHookScriptConfig: RequestHookScriptConfig = {
  enabled: false,
  language: "javascript",
  code: "",
};

const TEMPLATE_DELETE_CODEX_METADATA = `({
  onRequest: function (context, request) {
    // 删除可能包含非 ASCII 的请求头（例如含中文路径），避免被 CF 严格校验拦截
    delete request.headers["x-codex-turn-metadata"];

    // 放行
    return request;
  },

  onResponse: function (context, response) {
    // 示例：透传响应（可选）
    return response;
  }
})
`;

interface RequestHookScriptConfigProps {
  appId: string;
  providerId?: string;
  config: RequestHookScriptConfig;
  onConfigChange: (config: RequestHookScriptConfig) => void;
}

export function RequestHookScriptConfig({
  appId,
  providerId,
  config,
  onConfigChange,
}: RequestHookScriptConfigProps) {
  const { t } = useTranslation();
  const [isOpen, setIsOpen] = useState(config.enabled);
  const [isTestingOpen, setIsTestingOpen] = useState(false);
  const [testing, setTesting] = useState(false);

  const defaultTestHeaders = useMemo(
    () =>
      JSON.stringify(
        {
          "content-type": "application/json",
          "x-codex-turn-metadata":
            '{"workspaces":{"/Users/snakesammy/Desktop/项目/思考":{"latest_git_commit_hash":"381d3907cbd7f9ec3e4f7073decfd8f8c8861072"}}}',
        },
        null,
        2,
      ),
    [],
  );

  const [testHeaders, setTestHeaders] = useState(defaultTestHeaders);
  const [testBody, setTestBody] = useState(
    JSON.stringify({ model: "gpt-4.1", input: "ping" }, null, 2),
  );
  const [testResultHeaders, setTestResultHeaders] = useState<string>("");
  const [testResultBody, setTestResultBody] = useState<string>("");
  const [testResultUrl, setTestResultUrl] = useState<string>("");

  const canTest = Boolean(providerId) && appId === "codex";

  const handleInsertTemplate = () => {
    onConfigChange({
      ...config,
      enabled: true,
      language: "javascript",
      code: TEMPLATE_DELETE_CODEX_METADATA,
    });
    setIsOpen(true);
    toast.success(t("common.inserted", { defaultValue: "已插入模板" }));
  };

  const runTest = async () => {
    if (!canTest || !providerId) return;
    if (!config.code.trim()) {
      toast.error(
        t("provider.requestHookScriptEmpty", {
          defaultValue: "脚本为空，无法测试",
        }),
      );
      return;
    }

    let headersObj: Record<string, string>;
    let bodyObj: any;
    try {
      const parsed = JSON.parse(testHeaders || "{}");
      if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
        throw new Error("headers must be an object");
      }
      headersObj = {};
      for (const [k, v] of Object.entries(parsed)) {
        if (typeof v !== "string") {
          throw new Error(`header ${k} must be a string`);
        }
        headersObj[k] = v;
      }
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      toast.error(
        t("provider.requestHookScriptInvalidHeaders", {
          defaultValue: "测试 headers JSON 无效：{{error}}",
          error: message,
        }),
      );
      return;
    }

    try {
      bodyObj = JSON.parse(testBody || "null");
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      toast.error(
        t("provider.requestHookScriptInvalidBody", {
          defaultValue: "测试 body JSON 无效：{{error}}",
          error: message,
        }),
      );
      return;
    }

    setTesting(true);
    try {
      const result = await providersApi.testRequestHookScript(
        providerId,
        "codex",
        config.code,
        headersObj,
        bodyObj,
        "/v1/responses",
      );
      setTestResultUrl(result.url || "");
      setTestResultHeaders(JSON.stringify(result.headers ?? {}, null, 2));
      setTestResultBody(JSON.stringify(result.body ?? null, null, 2));
      toast.success(
        t("provider.requestHookScriptTestSuccess", {
          defaultValue: "测试成功",
        }),
      );
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      toast.error(
        t("provider.requestHookScriptTestFailed", {
          defaultValue: "测试失败：{{error}}",
          error: message,
        }),
        { duration: 6000 },
      );
    } finally {
      setTesting(false);
    }
  };

  const handleTestScriptClick = async () => {
    if (!canTest || testing) return;
    if (!isTestingOpen) {
      setIsTestingOpen(true);
    }
    await runTest();
  };

  return (
    <div className="space-y-3">
      <div
        className="flex items-center gap-2 cursor-pointer select-none"
        onClick={() => setIsOpen(!isOpen)}
      >
        {isOpen ? (
          <ChevronDown className="h-4 w-4 text-muted-foreground" />
        ) : (
          <ChevronRight className="h-4 w-4 text-muted-foreground" />
        )}
        <Code2 className="h-4 w-4 text-muted-foreground" />
        <span className="text-sm font-medium">
          {t(
            "provider.requestHookScript",
            "请求/响应重写脚本 (onRequest/onResponse)",
          )}
        </span>
        <div className="ml-auto" onClick={(e) => e.stopPropagation()}>
          <Switch
            checked={config.enabled}
            onCheckedChange={(checked) =>
              onConfigChange({ ...config, enabled: checked })
            }
          />
        </div>
      </div>

      {isOpen && config.enabled && (
        <div className="pl-6 space-y-3">
          <div className="flex flex-wrap items-center gap-2">
            <Button
              type="button"
              variant="outline"
              size="sm"
              className="h-7 text-xs"
              onClick={handleInsertTemplate}
            >
              <Wand2 className="h-3.5 w-3.5 mr-1" />
              {t("provider.insertTemplate", "插入模板")}
            </Button>
            <Button
              type="button"
              variant="outline"
              size="sm"
              className="h-7 text-xs"
              disabled={!canTest || testing}
              onClick={() => {
                void handleTestScriptClick();
              }}
              title={
                canTest
                  ? undefined
                  : t("provider.saveBeforeTest", {
                      defaultValue: "保存供应商后才能测试",
                    })
              }
            >
              <Play className="h-3.5 w-3.5 mr-1" />
              {t("provider.testScript", "测试脚本")}
            </Button>
          </div>

          <JsonEditor
            id="request-hook-script"
            value={config.code || ""}
            onChange={(value) => onConfigChange({ ...config, code: value })}
            height={360}
            language="javascript"
            showMinimap={false}
            completionMode="requestHookScript"
            placeholder={TEMPLATE_DELETE_CODEX_METADATA}
          />

          {isTestingOpen && (
            <div className="space-y-3">
              <div className="text-xs text-muted-foreground">
                {t("provider.requestHookScriptTestHint", {
                  defaultValue:
                    "下面的 headers/body 仅用于测试脚本输出，不会保存到配置。",
                })}
              </div>

              <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
                <div className="space-y-2">
                  <div className="text-xs font-medium">
                    {t("provider.testHeaders", "测试 Headers (JSON 对象)")}
                  </div>
                  <JsonEditor
                    id="request-hook-test-headers"
                    value={testHeaders}
                    onChange={setTestHeaders}
                    height={220}
                    language="json"
                    showMinimap={false}
                  />
                </div>
                <div className="space-y-2">
                  <div className="text-xs font-medium">
                    {t("provider.testBody", "测试 Body (JSON)")}
                  </div>
                  <JsonEditor
                    id="request-hook-test-body"
                    value={testBody}
                    onChange={setTestBody}
                    height={220}
                    language="json"
                    showMinimap={false}
                  />
                </div>
              </div>

              <div className="flex items-center gap-2">
                <Button
                  type="button"
                  size="sm"
                  disabled={!canTest || testing}
                  onClick={runTest}
                >
                  {testing
                    ? t("provider.testing", "测试中...")
                    : t("provider.runTest", "运行测试")}
                </Button>
              </div>

              {(testResultHeaders || testResultBody || testResultUrl) && (
                <div className="space-y-3">
                  {testResultUrl && (
                    <div className="space-y-1">
                      <div className="text-xs font-medium">
                        {t("provider.resultUrl", "输出 URL")}
                      </div>
                      <div className="text-xs rounded border px-2 py-1 font-mono break-all">
                        {testResultUrl}
                      </div>
                    </div>
                  )}
                  <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
                    <div className="space-y-2">
                      <div className="text-xs font-medium">
                        {t("provider.resultHeaders", "输出 Headers")}
                      </div>
                      <JsonEditor
                        id="request-hook-result-headers"
                        value={testResultHeaders}
                        onChange={setTestResultHeaders}
                        height={220}
                        language="json"
                        showMinimap={false}
                      />
                    </div>
                    <div className="space-y-2">
                      <div className="text-xs font-medium">
                        {t("provider.resultBody", "输出 Body")}
                      </div>
                      <JsonEditor
                        id="request-hook-result-body"
                        value={testResultBody}
                        onChange={setTestResultBody}
                        height={220}
                        language="json"
                        showMinimap={false}
                      />
                    </div>
                  </div>
                </div>
              )}
            </div>
          )}
        </div>
      )}
    </div>
  );
}
