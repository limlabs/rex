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
      { title: "First result for: " + params.query, score: 0.95 },
      { title: "Second result for: " + params.query, score: 0.8 },
      { title: "Third result for: " + params.query, score: 0.65 },
    ],
  };
}
