import nextVitals from "eslint-config-next/core-web-vitals";
import nextTs from "eslint-config-next/typescript";

const config = [
  { ignores: [".next/**", "node_modules/**", "public/mockServiceWorker.js", "src/lib/api/schema.d.ts", "next-env.d.ts", "playwright-report/**", "test-results/**"] },
  ...nextVitals,
  ...nextTs,
  {
    rules: {
      // A leading underscore marks a value that is taken out of an object on purpose.
      "@typescript-eslint/no-unused-vars": ["warn", { argsIgnorePattern: "^_", varsIgnorePattern: "^_", destructuredArrayIgnorePattern: "^_", ignoreRestSiblings: true }],
    },
  },
];

export default config;
