export const description = "Search items by query";

export const parameters = {
  type: "object",
  properties: {
    query: { type: "string", description: "Search query" },
  },
  required: ["query"],
};

export default async function (params: { query: string }) {
  return {
    results: [
      { title: "Result for: " + params.query, score: 0.95 },
    ],
  };
}
