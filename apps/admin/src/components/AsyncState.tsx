interface AsyncStateProps {
    title: string;
    description: string;
    error?: string | null;
    onRetry?: () => void;
}

export function LoadingState({ title, description }: AsyncStateProps) {
    return (
        <div>
            <div className="page-header">
                <div className="page-header-row">
                    <div>
                        <h1 className="page-title">{title}</h1>
                        <p className="page-description">{description}</p>
                    </div>
                </div>
            </div>
            <div className="card">
                <div className="card-content" style={{ padding: "var(--space-8)" }}>
                    Loading…
                </div>
            </div>
        </div>
    );
}

export function ErrorState({ title, description, error, onRetry }: AsyncStateProps) {
    return (
        <div>
            <div className="page-header">
                <div className="page-header-row">
                    <div>
                        <h1 className="page-title">{title}</h1>
                        <p className="page-description">{description}</p>
                    </div>
                </div>
            </div>
            <div className="card">
                <div className="card-content" style={{ padding: "var(--space-8)" }}>
                    <div className="alert alert-error" style={{ display: "grid", gap: "var(--space-3)" }}>
                        <strong>Request failed</strong>
                        <code>{error || "Unknown error"}</code>
                        {onRetry ? (
                            <div>
                                <button className="btn btn-primary" onClick={onRetry}>
                                    Retry
                                </button>
                            </div>
                        ) : null}
                    </div>
                </div>
            </div>
        </div>
    );
}
