import { useEffect, useState } from "react";
import { formatJson } from "../lib/formatters";

interface JsonEditorCardProps {
    title: string;
    description: string;
    value: unknown;
    saveLabel?: string;
    disabled?: boolean;
    onSave?: (value: unknown) => Promise<void>;
}

function JsonEditorCard({
    title,
    description,
    value,
    saveLabel = "Save",
    disabled = false,
    onSave,
}: JsonEditorCardProps) {
    const [draft, setDraft] = useState(formatJson(value));
    const [error, setError] = useState<string | null>(null);
    const [saving, setSaving] = useState(false);

    useEffect(() => {
        setDraft(formatJson(value));
        setError(null);
    }, [value]);

    const handleSave = async () => {
        if (!onSave) {
            return;
        }

        try {
            setSaving(true);
            setError(null);
            const parsed = JSON.parse(draft);
            await onSave(parsed);
        } catch (err) {
            setError(err instanceof Error ? err.message : "Failed to save JSON");
        } finally {
            setSaving(false);
        }
    };

    return (
        <div className="card">
            <div className="card-header">
                <div>
                    <h3 className="card-title">{title}</h3>
                    <p className="card-description">{description}</p>
                </div>
                <div style={{ display: "flex", gap: "var(--space-2)" }}>
                    <button
                        className="btn btn-secondary"
                        onClick={() => setDraft(formatJson(value))}
                    >
                        Reset
                    </button>
                    {onSave ? (
                        <button
                            className="btn btn-primary"
                            onClick={handleSave}
                            disabled={disabled || saving}
                        >
                            {saving ? "Saving…" : saveLabel}
                        </button>
                    ) : null}
                </div>
            </div>
            <div className="card-content">
                <textarea
                    className="form-input form-textarea"
                    value={draft}
                    onChange={(event) => setDraft(event.target.value)}
                    disabled={disabled}
                    style={{ minHeight: 360, fontFamily: "monospace" }}
                />
                {error ? (
                    <div className="alert alert-error" style={{ marginTop: "var(--space-3)" }}>
                        <code>{error}</code>
                    </div>
                ) : null}
            </div>
        </div>
    );
}

export default JsonEditorCard;
