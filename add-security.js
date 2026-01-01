#!/usr/bin/env node
const fs = require('fs');
const path = require('path');

const handlersDir = path.join(__dirname, 'src/handlers');
const files = fs.readdirSync(handlersDir).filter(f => f.endsWith('.rs') && f !== 'dashboard.rs');

for (const file of files) {
    const filePath = path.join(handlersDir, file);
    let content = fs.readFileSync(filePath, 'utf8');

    // Match utoipa::path blocks without security and add it
    const pattern = /#\[utoipa::path\(\s*\n\s*(get|post|put|delete|patch),\s*\n\s*(path\s*=)/g;

    let modified = content.replace(pattern, (match, method, pathPart) => {
        // Don't duplicate if already has security
        return `#[utoipa::path(\n    ${method},\n    security(("bearer_auth" = [])),\n    ${pathPart}`;
    });

    // Remove duplicate security lines
    modified = modified.replace(/security\(\("bearer_auth" = \[\]\)\),\s*\n\s*security\(\("bearer_auth" = \[\]\)\),/g,
        'security(("bearer_auth" = [])),');

    if (modified !== content) {
        fs.writeFileSync(filePath, modified);
        console.log(`Updated: ${file}`);
    } else {
        console.log(`Skipped: ${file}`);
    }
}

console.log('Done!');
