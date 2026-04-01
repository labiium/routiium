import { BrowserRouter, Routes, Route } from "react-router-dom";
import Layout from "./components/Layout";
import Dashboard from "./pages/Dashboard";
import ApiKeys from "./pages/ApiKeys";
import Routing from "./pages/Routing";
import RateLimiting from "./pages/RateLimiting";
import Analytics from "./pages/Analytics";
import SystemPrompts from "./pages/SystemPrompts";
import Users from "./pages/Users";
import Settings from "./pages/Settings";
import Mcp from "./pages/mcp/Mcp";
import Pricing from "./pages/pricing/Pricing";
import Bedrock from "./pages/bedrock/Bedrock";
import ChatHistory from "./pages/chatHistory/ChatHistory";

function App() {
    return (
        <BrowserRouter>
            <Routes>
                <Route path="/" element={<Layout />}>
                    <Route index element={<Dashboard />} />
                    <Route path="api-keys" element={<ApiKeys />} />
                    <Route path="routing" element={<Routing />} />
                    <Route path="rate-limiting" element={<RateLimiting />} />
                    <Route path="analytics" element={<Analytics />} />
                    <Route path="system-prompts" element={<SystemPrompts />} />
                    <Route path="users" element={<Users />} />
                    <Route path="settings" element={<Settings />} />
                    <Route path="mcp" element={<Mcp />} />
                    <Route path="pricing" element={<Pricing />} />
                    <Route path="bedrock" element={<Bedrock />} />
                    <Route path="chat-history" element={<ChatHistory />} />
                </Route>
            </Routes>
        </BrowserRouter>
    );
}

export default App;
