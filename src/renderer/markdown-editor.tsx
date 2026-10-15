import React, { useEffect, useRef, useState } from "react";
import {
  EditorContent,
  NodeViewWrapper,
  ReactNodeViewRenderer,
  useEditor,
  type Editor,
  type NodeViewProps,
} from "@tiptap/react";
import StarterKit from "@tiptap/starter-kit";
import Image from "@tiptap/extension-image";
import Placeholder from "@tiptap/extension-placeholder";
import { Markdown } from "@tiptap/markdown";
import type { Draft, MarkdownAsset } from "../shared/domain";

const assetScheme = "abd-asset://";

const ManagedImage = Image.extend({
  addAttributes() {
    return {
      ...this.parent?.(),
      resourceId: {
        default: null,
        rendered: false,
      },
    };
  },
  parseMarkdown(token, helpers) {
    const source = typeof token.href === "string" ? token.href : "";
    const resourceId = source.startsWith(assetScheme)
      ? source.slice(assetScheme.length)
      : null;
    return helpers.createNode("image", {
      src: resourceId ? "" : source,
      resourceId,
      title: token.title,
      alt: token.text,
    });
  },
  renderMarkdown(node) {
    const resourceId = String(node.attrs?.resourceId ?? "");
    const source = resourceId
      ? `${assetScheme}${resourceId}`
      : String(node.attrs?.src ?? "");
    const alt = String(node.attrs?.alt ?? "").replaceAll("]", "\\]");
    const title = String(node.attrs?.title ?? "").replaceAll('"', '\\"');
    return title ? `![${alt}](${source} "${title}")` : `![${alt}](${source})`;
  },
});

function EditableImageNodeView({
  node,
  deleteNode,
  selected,
  editor,
}: NodeViewProps) {
  const alt = String(node.attrs.alt ?? "").trim();
  const label = alt ? `移除图片 ${alt}` : "移除图片";
  return (
    <NodeViewWrapper
      className={`markdown-image-node${selected ? " selected" : ""}`}
      contentEditable={false}
    >
      <img
        src={String(node.attrs.src ?? "")}
        alt={alt}
        title={String(node.attrs.title ?? "")}
      />
      {editor.isEditable && (
        <button
          type="button"
          className="markdown-image-remove"
          aria-label={label}
          title="从编辑内容中移除图片"
          onMouseDown={(event) => event.preventDefault()}
          onClick={(event) => {
            event.preventDefault();
            event.stopPropagation();
            if (!editor.isEditable) return;
            deleteNode();
            editor.commands.focus();
          }}
        >
          <span aria-hidden="true">×</span>
        </button>
      )}
    </NodeViewWrapper>
  );
}

const EditableManagedImage = ManagedImage.extend({
  addNodeView() {
    return ReactNodeViewRenderer(EditableImageNodeView);
  },
});

export interface MarkdownImageImportResult {
  draft: Draft;
  asset: MarkdownAsset;
  previewDataUrl: string;
}

interface MarkdownEditorProps {
  value: string;
  placeholder: string;
  disabled?: boolean;
  required?: boolean;
  onChange: (value: string) => void;
  onPasteImage: (file: File | null) => Promise<MarkdownImageImportResult>;
  onReadAsset: (assetId: string) => Promise<string>;
}

function fileToBase64(file: File) {
  return file.arrayBuffer().then((buffer) => {
    const bytes = new Uint8Array(buffer);
    let binary = "";
    const chunkSize = 32_768;
    for (let index = 0; index < bytes.length; index += chunkSize) {
      binary += String.fromCharCode(
        ...bytes.subarray(index, index + chunkSize),
      );
    }
    return btoa(binary);
  });
}

export async function markdownImageFilePayload(file: File | null) {
  if (!file) return { name: null, dataBase64: null };
  return {
    name: file.name || "pasted-image",
    dataBase64: await fileToBase64(file),
  };
}

function resourceIdFromNode(node: { attrs?: Record<string, unknown> }) {
  const explicit = String(node.attrs?.resourceId ?? "");
  if (explicit) return explicit;
  const source = String(node.attrs?.src ?? "");
  return source.startsWith(assetScheme) ? source.slice(assetScheme.length) : "";
}

function replacePreviewSource(
  editor: Editor,
  resourceId: string,
  source: string,
) {
  const transaction = editor.state.tr;
  let changed = false;
  editor.state.doc.descendants((node, position) => {
    if (node.type.name !== "image" || resourceIdFromNode(node) !== resourceId)
      return;
    transaction.setNodeMarkup(position, undefined, {
      ...node.attrs,
      resourceId,
      src: source,
    });
    changed = true;
  });
  if (changed) editor.view.dispatch(transaction.setMeta("addToHistory", false));
}

function imageFromPaste(event: ClipboardEvent) {
  const files = Array.from(event.clipboardData?.files ?? []);
  const file = files.find((candidate) => candidate.type.startsWith("image/"));
  if (file) return file;
  for (const item of Array.from(event.clipboardData?.items ?? [])) {
    if (item.kind === "file" && item.type.startsWith("image/")) {
      const candidate = item.getAsFile();
      if (candidate) return candidate;
    }
  }
  return null;
}

function shouldTryNativeClipboard(event: ClipboardEvent) {
  const data = event.clipboardData;
  if (!data) return true;
  if (data.files.length > 0) return true;
  const plain = data.getData("text/plain").trim();
  return (
    plain === "" ||
    plain.startsWith("file://") ||
    /^[A-Za-z]:\\.*\.(png|jpe?g)$/iu.test(plain) ||
    /^\/.*\.(png|jpe?g)$/iu.test(plain)
  );
}

export function MarkdownEditor({
  value,
  placeholder,
  disabled = false,
  required = false,
  onChange,
  onPasteImage,
  onReadAsset,
}: MarkdownEditorProps) {
  const callbacks = useRef({ onChange, onPasteImage, onReadAsset });
  callbacks.current = { onChange, onPasteImage, onReadAsset };
  const emitted = useRef(value);
  const previews = useRef(new Map<string, string>());
  const [pasteError, setPasteError] = useState("");
  const editor = useEditor({
    immediatelyRender: false,
    content: value,
    contentType: "markdown",
    editable: !disabled,
    extensions: [
      StarterKit.configure({ link: { openOnClick: false } }),
      EditableManagedImage.configure({ inline: false, allowBase64: false }),
      Placeholder.configure({ placeholder }),
      Markdown,
    ],
    editorProps: {
      attributes: {
        class: "markdown-editor-content",
        role: "textbox",
        "aria-multiline": "true",
        "aria-required": required ? "true" : "false",
        "aria-placeholder": placeholder,
      },
      handlePaste: (view, event) => {
        const file = imageFromPaste(event);
        if (!file && !shouldTryNativeClipboard(event)) return false;
        event.preventDefault();
        const position = view.state.selection.from;
        setPasteError("");
        void callbacks.current
          .onPasteImage(file)
          .then(({ asset, previewDataUrl }) => {
            previews.current.set(asset.id, previewDataUrl);
            editor
              ?.chain()
              .focus()
              .insertContentAt(position, {
                type: "image",
                attrs: {
                  src: previewDataUrl,
                  resourceId: asset.id,
                  alt: asset.name,
                  title: asset.name,
                },
              })
              .run();
          })
          .catch((error) => setPasteError(String(error)));
        return true;
      },
    },
    onUpdate: ({ editor: current }) => {
      const markdown = current.getMarkdown();
      if (markdown === emitted.current) return;
      emitted.current = markdown;
      callbacks.current.onChange(markdown);
    },
  });

  useEffect(() => {
    callbacks.current = { onChange, onPasteImage, onReadAsset };
  }, [onChange, onPasteImage, onReadAsset]);

  useEffect(() => {
    editor?.setEditable(!disabled);
  }, [disabled, editor]);

  useEffect(() => {
    if (!editor || value === emitted.current) return;
    emitted.current = value;
    editor.commands.setContent(value, {
      contentType: "markdown",
      emitUpdate: false,
    });
  }, [editor, value]);

  useEffect(() => {
    if (!editor) return;
    const resourceIds = new Set<string>();
    editor.state.doc.descendants((node) => {
      if (node.type.name === "image") {
        const id = resourceIdFromNode(node);
        if (id) resourceIds.add(id);
      }
    });
    for (const resourceId of resourceIds) {
      const cached = previews.current.get(resourceId);
      if (cached) {
        replacePreviewSource(editor, resourceId, cached);
        continue;
      }
      void callbacks.current
        .onReadAsset(resourceId)
        .then((source) => {
          previews.current.set(resourceId, source);
          replacePreviewSource(editor, resourceId, source);
        })
        .catch((error) => setPasteError(String(error)));
    }
  }, [editor, value]);

  if (!editor)
    return <div className="markdown-editor loading">正在载入编辑器…</div>;

  const toolbarButton = (
    label: string,
    active: boolean,
    action: () => void,
    unavailable = false,
  ) => (
    <button
      type="button"
      className={active ? "active" : ""}
      aria-label={label}
      aria-pressed={active}
      disabled={disabled || unavailable}
      onMouseDown={(event) => event.preventDefault()}
      onClick={action}
    >
      {label}
    </button>
  );

  return (
    <div className={`markdown-editor${pasteError ? " has-error" : ""}`}>
      <div
        className="markdown-toolbar"
        role="toolbar"
        aria-label="Markdown 格式"
      >
        {toolbarButton(
          "撤销",
          false,
          () => editor.chain().focus().undo().run(),
          !editor.can().undo(),
        )}
        {toolbarButton(
          "重做",
          false,
          () => editor.chain().focus().redo().run(),
          !editor.can().redo(),
        )}
        {toolbarButton(
          "二级标题",
          editor.isActive("heading", { level: 2 }),
          () => editor.chain().focus().toggleHeading({ level: 2 }).run(),
        )}
        {toolbarButton("粗体", editor.isActive("bold"), () =>
          editor.chain().focus().toggleBold().run(),
        )}
        {toolbarButton("斜体", editor.isActive("italic"), () =>
          editor.chain().focus().toggleItalic().run(),
        )}
        {toolbarButton("无序列表", editor.isActive("bulletList"), () =>
          editor.chain().focus().toggleBulletList().run(),
        )}
        {toolbarButton("有序列表", editor.isActive("orderedList"), () =>
          editor.chain().focus().toggleOrderedList().run(),
        )}
        {toolbarButton("引用", editor.isActive("blockquote"), () =>
          editor.chain().focus().toggleBlockquote().run(),
        )}
        {toolbarButton("代码", editor.isActive("code"), () =>
          editor.chain().focus().toggleCode().run(),
        )}
      </div>
      <EditorContent editor={editor} />
      {pasteError && <small className="error-text">{pasteError}</small>}
    </div>
  );
}

interface MarkdownPreviewProps {
  value: string;
  onReadAsset: (assetId: string) => Promise<string>;
}

export function MarkdownPreview({ value, onReadAsset }: MarkdownPreviewProps) {
  const previews = useRef(new Map<string, string>());
  const readAsset = useRef(onReadAsset);
  readAsset.current = onReadAsset;
  const editor = useEditor({
    immediatelyRender: false,
    content: value,
    contentType: "markdown",
    editable: false,
    extensions: [
      StarterKit.configure({ link: { openOnClick: true } }),
      ManagedImage.configure({ inline: false, allowBase64: false }),
      Markdown,
    ],
    editorProps: {
      attributes: {
        class: "markdown-preview-content",
      },
    },
  });

  useEffect(() => {
    if (!editor) return;
    editor.commands.setContent(value, {
      contentType: "markdown",
      emitUpdate: false,
    });
    const resourceIds = new Set<string>();
    editor.state.doc.descendants((node) => {
      if (node.type.name !== "image") return;
      const id = resourceIdFromNode(node);
      if (id) resourceIds.add(id);
    });
    for (const id of resourceIds) {
      const cached = previews.current.get(id);
      if (cached) {
        replacePreviewSource(editor, id, cached);
        continue;
      }
      void readAsset
        .current(id)
        .then((source) => {
          previews.current.set(id, source);
          replacePreviewSource(editor, id, source);
        })
        .catch(() => undefined);
    }
  }, [editor, value]);

  if (!editor) return null;
  return <EditorContent editor={editor} />;
}
