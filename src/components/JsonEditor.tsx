import React, { useRef, useEffect, useMemo, useState } from "react";
import { EditorView, basicSetup } from "codemirror";
import { json } from "@codemirror/lang-json";
import { javascript } from "@codemirror/lang-javascript";
import {
  HighlightStyle,
  syntaxHighlighting,
  foldGutter,
  indentOnInput,
  bracketMatching,
  foldKeymap,
} from "@codemirror/language";
import {
  autocompletion,
  closeBrackets,
  closeBracketsKeymap,
  completionKeymap,
  type CompletionContext,
} from "@codemirror/autocomplete";
import { oneDark } from "@codemirror/theme-one-dark";
import { EditorState } from "@codemirror/state";
import { Prec } from "@codemirror/state";
import { placeholder } from "@codemirror/view";
import { linter, lintKeymap, type Diagnostic } from "@codemirror/lint";
import { tags as t } from "@lezer/highlight";
import { useTranslation } from "react-i18next";
import { Wand2 } from "lucide-react";
import { toast } from "sonner";
import { formatJSON } from "@/utils/formatters";
import {
  lineNumbers,
  highlightActiveLineGutter,
  highlightSpecialChars,
  drawSelection,
  dropCursor,
  rectangularSelection,
  crosshairCursor,
  highlightActiveLine,
  keymap,
} from "@codemirror/view";
import { history, defaultKeymap, historyKeymap } from "@codemirror/commands";
import { highlightSelectionMatches, searchKeymap } from "@codemirror/search";

interface JsonEditorProps {
  id?: string;
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
  darkMode?: boolean;
  rows?: number;
  showValidation?: boolean;
  language?: "json" | "javascript";
  height?: string | number;
  showMinimap?: boolean; // 添加此属性以防未来使用
  completionMode?: "requestHookScript";
}

const requestHookScriptHighlight = HighlightStyle.define([
  { tag: [t.keyword, t.modifier], color: "#FF7A90", fontWeight: "700" },
  {
    tag: [
      t.function(t.variableName),
      t.function(t.propertyName),
      t.labelName,
      t.definition(t.propertyName),
      t.special(t.propertyName),
      t.name,
    ],
    color: "#7FD6FF",
  },
  {
    tag: [
      t.definition(t.variableName),
      t.local(t.variableName),
      t.variableName,
      t.propertyName,
      t.special(t.variableName),
      t.standard(t.variableName),
      t.attributeName,
      t.atom,
      t.url,
    ],
    color: "#AFC7FF",
  },
  {
    tag: [t.typeName, t.className, t.namespace, t.macroName],
    color: "#D7B8FF",
  },
  {
    tag: [t.string, t.special(t.string), t.inserted],
    color: "#A8E6A3",
  },
  { tag: [t.number, t.bool, t.null, t.literal], color: "#FFBD6E" },
  { tag: [t.regexp, t.escape], color: "#FF9670" },
  { tag: [t.deleted, t.invalid], color: "#FF5D73" },
  { tag: t.comment, color: "#9AA4B2" },
  {
    tag: [t.operator, t.punctuation, t.separator, t.contentSeparator],
    color: "#E2E5EC",
  },
]);

const requestHookScriptTheme = EditorView.theme(
  {
    "&": {
      backgroundColor: "#11141B",
      color: "#F2F5FA",
    },
    ".cm-content": {
      color: "#F2F5FA",
      caretColor: "#FFD479",
    },
    ".cm-cursor, .cm-dropCursor": {
      borderLeftColor: "#FFD479",
    },
    ".cm-selectionBackground, &.cm-focused .cm-selectionBackground": {
      backgroundColor: "rgba(255, 212, 121, 0.30) !important",
    },
    ".cm-activeLine": {
      backgroundColor: "rgba(255, 212, 121, 0.08) !important",
    },
    ".cm-activeLineGutter": {
      backgroundColor: "rgba(255, 212, 121, 0.08) !important",
    },
    ".cm-gutters": {
      color: "#9AA0AD",
      backgroundColor: "#11141B",
      borderRight: "1px solid rgba(154, 160, 173, 0.22)",
    },
  },
  { dark: true },
);

// RequestHookScript 模式下不使用 basicSetup，避免默认 defaultHighlightStyle 引入蓝色 token
const requestHookScriptSetup = [
  lineNumbers(),
  highlightActiveLineGutter(),
  highlightSpecialChars(),
  history(),
  foldGutter(),
  drawSelection(),
  dropCursor(),
  EditorState.allowMultipleSelections.of(true),
  indentOnInput(),
  bracketMatching(),
  closeBrackets(),
  rectangularSelection(),
  crosshairCursor(),
  highlightActiveLine(),
  highlightSelectionMatches(),
  keymap.of([
    ...closeBracketsKeymap,
    ...defaultKeymap,
    ...searchKeymap,
    ...historyKeymap,
    ...foldKeymap,
    ...completionKeymap,
    ...lintKeymap,
  ]),
];

function requestHookScriptCompletionSource(context: CompletionContext) {
  const word = context.matchBefore(/[A-Za-z_][A-Za-z0-9_\.]*/);
  if (!word && !context.explicit) return null;

  const text = word?.text ?? "";
  const pos = context.pos;
  const from = word?.from ?? pos;

  const baseOptions = [
    {
      label: "context",
      type: "variable",
      info: "onRequest/onResponse 的上下文（只读）",
    },
    {
      label: "request",
      type: "variable",
      info: "即将发往上游的请求视图（onRequest，可修改）",
    },
    {
      label: "response",
      type: "variable",
      info: "返回给客户端的响应视图（onResponse，可修改）",
    },
    {
      label: 'delete request.headers["x-codex-turn-metadata"];',
      type: "snippet",
      apply: 'delete request.headers["x-codex-turn-metadata"];',
      info: "删除可能包含非 ASCII 的请求头（例如含中文路径）",
    },
    {
      label: "return request;",
      type: "snippet",
      apply: "return request;",
      info: "放行请求（必须 return request）",
    },
    {
      label: "return response;",
      type: "snippet",
      apply: "return response;",
      info: "放行响应（必须 return response）",
    },
  ];

  const contextPrefix = "context.";
  if (text.startsWith(contextPrefix)) {
    const propPrefix = text.slice(contextPrefix.length);
    const options = [
      { label: "app", type: "property", info: "应用 ID（如 codex）" },
      { label: "method", type: "property", info: "请求方法（如 POST）" },
      { label: "path", type: "property", info: "请求路径（不含域名）" },
      { label: "endpoint", type: "property", info: "端点（如 /v1/responses）" },
      { label: "url", type: "property", info: "将要请求的 URL" },
      { label: "provider", type: "property", info: "当前 Provider 信息" },
      {
        label: "incomingHeaders",
        type: "property",
        info: "入站原始请求头（只读）",
      },
    ].filter((o) => o.label.startsWith(propPrefix));
    return {
      from: (word?.from ?? pos) + contextPrefix.length,
      options,
      validFor: /^[A-Za-z_][A-Za-z0-9_]*$/,
    };
  }

  const requestPrefix = "request.";
  if (text.startsWith(requestPrefix)) {
    const propPrefix = text.slice(requestPrefix.length);
    const options = [
      { label: "headers", type: "property", info: "最终将发往上游的请求头" },
      { label: "queries", type: "property", info: "最终将发往上游的查询参数" },
      { label: "body", type: "property", info: "最终将发往上游的请求体" },
    ].filter((o) => o.label.startsWith(propPrefix));
    return {
      from: (word?.from ?? pos) + requestPrefix.length,
      options,
      validFor: /^[A-Za-z_][A-Za-z0-9_]*$/,
    };
  }

  const responsePrefix = "response.";
  if (text.startsWith(responsePrefix)) {
    const propPrefix = text.slice(responsePrefix.length);
    const options = [
      { label: "code", type: "property", info: "最终返回给客户端的状态码" },
      { label: "headers", type: "property", info: "最终返回给客户端的响应头" },
      { label: "body", type: "property", info: "最终返回给客户端的响应体" },
    ].filter((o) => o.label.startsWith(propPrefix));
    return {
      from: (word?.from ?? pos) + responsePrefix.length,
      options,
      validFor: /^[A-Za-z_][A-Za-z0-9_]*$/,
    };
  }

  return {
    from,
    options: baseOptions.filter((o) => o.label.startsWith(text)),
  };
}

const JsonEditor: React.FC<JsonEditorProps> = ({
  value,
  onChange,
  placeholder: placeholderText = "",
  darkMode,
  rows = 12,
  showValidation = true,
  language = "json",
  height,
  completionMode,
}) => {
  const { t } = useTranslation();
  const editorRef = useRef<HTMLDivElement>(null);
  const viewRef = useRef<EditorView | null>(null);
  const [autoDarkMode, setAutoDarkMode] = useState(false);

  const isRequestHookScript =
    language === "javascript" && completionMode === "requestHookScript";
  const effectiveDarkMode = darkMode ?? autoDarkMode;

  useEffect(() => {
    const root = document.documentElement;
    const updateDarkMode = () => {
      setAutoDarkMode(root.classList.contains("dark"));
    };

    updateDarkMode();
    const observer = new MutationObserver(updateDarkMode);
    observer.observe(root, { attributes: true, attributeFilter: ["class"] });

    return () => observer.disconnect();
  }, []);

  // JSON linter 函数
  const jsonLinter = useMemo(
    () =>
      linter((view) => {
        const diagnostics: Diagnostic[] = [];
        if (!showValidation || language !== "json") return diagnostics;

        const doc = view.state.doc.toString();
        if (!doc.trim()) return diagnostics;

        try {
          const parsed = JSON.parse(doc);
          if (!(parsed && typeof parsed === "object" && !Array.isArray(parsed))) {
            diagnostics.push({
              from: 0,
              to: doc.length,
              severity: "error",
              message: t("jsonEditor.mustBeObject"),
            });
          }
        } catch (e) {
          const message =
            e instanceof SyntaxError ? e.message : t("jsonEditor.invalidJson");
          diagnostics.push({
            from: 0,
            to: doc.length,
            severity: "error",
            message,
          });
        }

        return diagnostics;
      }),
    [showValidation, language, t],
  );

  useEffect(() => {
    if (!editorRef.current) return;

    const minHeightPx = height ? undefined : Math.max(1, rows) * 18;
    const heightValue = height
      ? typeof height === "number"
        ? `${height}px`
        : height
      : undefined;

    const baseTheme = EditorView.baseTheme({
      ".cm-editor": {
        border: "1px solid hsl(var(--border))",
        borderRadius: "0.5rem",
        background: "transparent",
      },
      ".cm-editor.cm-focused": {
        outline: "none",
        borderColor: "hsl(var(--primary))",
      },
      ".cm-scroller": {
        background: "transparent",
      },
      ".cm-gutters": {
        background: "transparent",
        borderRight: "1px solid hsl(var(--border))",
        color: "hsl(var(--muted-foreground))",
      },
      ".cm-selectionBackground, .cm-content ::selection": {
        background: "hsl(var(--primary) / 0.12)",
      },
      ".cm-selectionMatch": {
        background: "hsl(var(--primary) / 0.08)",
      },
      ".cm-activeLine": {
        background: "hsl(var(--primary) / 0.04)",
      },
      ".cm-activeLineGutter": {
        background: "hsl(var(--primary) / 0.04)",
      },
    });

    const sizingTheme = EditorView.theme({
      "&": heightValue
        ? { height: heightValue }
        : { minHeight: `${minHeightPx}px` },
      ".cm-scroller": { overflow: "auto" },
      ".cm-content": {
        fontFamily:
          "ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, 'Liberation Mono', 'Courier New', monospace",
        fontSize: "14px",
      },
    });

    const extensions = [
      ...(isRequestHookScript ? requestHookScriptSetup : [basicSetup]),
      language === "javascript" ? javascript() : json(),
      placeholder(placeholderText || ""),
      baseTheme,
      sizingTheme,
      jsonLinter,
      ...(isRequestHookScript
        ? [
            oneDark,
            autocompletion({
              override: [requestHookScriptCompletionSource],
            }),
            requestHookScriptTheme,
            Prec.high(syntaxHighlighting(requestHookScriptHighlight)),
          ]
        : []),
      EditorView.updateListener.of((update) => {
        if (update.docChanged) {
          const newValue = update.state.doc.toString();
          onChange(newValue);
        }
      }),
    ];

    if (!isRequestHookScript && effectiveDarkMode) {
      extensions.push(oneDark);
    }

    const state = EditorState.create({
      doc: value,
      extensions,
    });

    const view = new EditorView({
      state,
      parent: editorRef.current,
    });

    viewRef.current = view;

    return () => {
      view.destroy();
      viewRef.current = null;
    };
  }, [
    effectiveDarkMode,
    rows,
    height,
    language,
    completionMode,
    isRequestHookScript,
    jsonLinter,
  ]);

  useEffect(() => {
    if (viewRef.current && viewRef.current.state.doc.toString() !== value) {
      const transaction = viewRef.current.state.update({
        changes: {
          from: 0,
          to: viewRef.current.state.doc.length,
          insert: value,
        },
      });
      viewRef.current.dispatch(transaction);
    }
  }, [value]);

  const handleFormat = () => {
    if (!viewRef.current) return;

    const currentValue = viewRef.current.state.doc.toString();
    if (!currentValue.trim()) return;

    try {
      const formatted = formatJSON(currentValue);
      onChange(formatted);
      toast.success(t("common.formatSuccess", { defaultValue: "格式化成功" }), {
        closeButton: true,
      });
    } catch (error) {
      const errorMessage =
        error instanceof Error ? error.message : String(error);
      toast.error(
        t("common.formatError", {
          defaultValue: "格式化失败：{{error}}",
          error: errorMessage,
        }),
      );
    }
  };

  const isFullHeight = height === "100%";

  return (
    <div
      style={{ width: "100%", height: isFullHeight ? "100%" : "auto" }}
      className={isFullHeight ? "flex flex-col" : ""}
    >
      <div
        ref={editorRef}
        style={{ width: "100%", height: isFullHeight ? undefined : "auto" }}
        className={isFullHeight ? "flex-1 min-h-0" : ""}
      />
      {language === "json" && (
        <button
          type="button"
          onClick={handleFormat}
          className={`${isFullHeight ? "mt-2 flex-shrink-0" : "mt-2"} inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium text-gray-700 dark:text-gray-300 hover:text-blue-600 dark:hover:text-blue-400 transition-colors`}
        >
          <Wand2 className="w-3.5 h-3.5" />
          {t("common.format", { defaultValue: "格式化" })}
        </button>
      )}
    </div>
  );
};

export default JsonEditor;
