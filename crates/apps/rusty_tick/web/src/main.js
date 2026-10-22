import { jsx as _jsx } from "react/jsx-runtime";
import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { App } from './app/App';
import { adoptIdentity, getMode, getToken } from './app/env';
import './styles/index.css';
if (getMode() === 'server')
    adoptIdentity(getToken()); // before any store reads its cache
createRoot(document.getElementById('root')).render(_jsx(StrictMode, { children: _jsx(App, {}) }));
