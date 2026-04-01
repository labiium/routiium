import { Outlet, NavLink, useLocation } from "react-router-dom";
import {
    BarChart3,
    Bot,
    Cloud,
    DollarSign,
    Gauge,
    Key,
    LayoutDashboard,
    MessageSquare,
    Route,
    Server,
    Settings,
    Users,
} from "lucide-react";
import ConnectionConfig from "./ConnectionConfig";

const navigation = [
    { name: "Dashboard", href: "/", icon: LayoutDashboard },
    { name: "API Keys", href: "/api-keys", icon: Key },
    { name: "Routing", href: "/routing", icon: Route },
    { name: "Rate Limiting", href: "/rate-limiting", icon: Gauge },
    { name: "Analytics", href: "/analytics", icon: BarChart3 },
    { name: "System Prompts", href: "/system-prompts", icon: Bot },
    { name: "MCP", href: "/mcp", icon: Server },
    { name: "Pricing", href: "/pricing", icon: DollarSign },
    { name: "Bedrock", href: "/bedrock", icon: Cloud },
    { name: "Chat History", href: "/chat-history", icon: MessageSquare },
    { name: "Principals", href: "/users", icon: Users },
    { name: "Settings", href: "/settings", icon: Settings },
];

const pageTitles: Record<string, string> = {
    "/": "Dashboard",
    "/api-keys": "API Keys",
    "/routing": "Routing",
    "/rate-limiting": "Rate Limiting",
    "/analytics": "Analytics",
    "/system-prompts": "System Prompts",
    "/mcp": "MCP",
    "/pricing": "Pricing",
    "/bedrock": "Bedrock",
    "/chat-history": "Chat History",
    "/users": "Principals",
    "/settings": "Settings",
};

function Layout() {
    const location = useLocation();
    const pageTitle = pageTitles[location.pathname] || "Dashboard";

    return (
        <div className="app-layout">
            <aside className="app-sidebar">
                <div className="sidebar-logo">
                    <a href="/">
                        <div className="logo-icon">R</div>
                        <span>Routiium</span>
                    </a>
                </div>

                <nav className="sidebar-nav">
                    {navigation.map((item) => (
                        <NavLink
                            key={item.name}
                            to={item.href}
                            className={({ isActive }) =>
                                `sidebar-link ${isActive ? "active" : ""}`
                            }
                            end={item.href === "/"}
                        >
                            <item.icon />
                            <span>{item.name}</span>
                        </NavLink>
                    ))}
                </nav>
            </aside>

            <main className="app-main">
                <header className="app-header">
                    <div className="header-left">
                        <div className="header-breadcrumb">
                            <span>Routiium</span>
                            <span>/</span>
                            <span>{pageTitle}</span>
                        </div>
                    </div>

                    <div className="header-right" style={{ width: "100%", justifyContent: "flex-end" }}>
                        <ConnectionConfig />
                    </div>
                </header>

                <div className="app-content">
                    <Outlet />
                </div>
            </main>
        </div>
    );
}

export default Layout;
