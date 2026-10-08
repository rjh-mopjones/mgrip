// The wiki's editor: CodeMirror with vim keys, bundled by the site build
// into dist/assets/editor.js and loaded by wiki.js when Edit is pressed.
//
// jk leaves insert mode, as Esc does. :w saves and :q closes, as in vim.

import { defaultKeymap, history, historyKeymap } from "@codemirror/commands";
import { markdown } from "@codemirror/lang-markdown";
import { EditorState } from "@codemirror/state";
import {
	drawSelection,
	EditorView,
	highlightActiveLine,
	keymap,
	lineNumbers,
} from "@codemirror/view";
import { Vim, vim } from "@replit/codemirror-vim";

export interface Editor {
	view: EditorView;
	text(): string;
}

export function openEditor(
	parent: HTMLElement,
	text: string,
	actions: { save: () => void; close: () => void },
): Editor {
	Vim.map("jk", "<Esc>", "insert");
	Vim.defineEx("write", "w", actions.save);
	Vim.defineEx("quit", "q", actions.close);
	Vim.defineEx("wq", "wq", () => actions.save());
	const view = new EditorView({
		parent,
		state: EditorState.create({
			doc: text,
			extensions: [
				// vim first, so its keys win over the default keymap.
				vim(),
				lineNumbers(),
				history(),
				drawSelection(),
				highlightActiveLine(),
				markdown(),
				EditorView.lineWrapping,
				keymap.of([...defaultKeymap, ...historyKeymap]),
			],
		}),
	});
	view.focus();
	return { view, text: () => view.state.doc.toString() };
}
