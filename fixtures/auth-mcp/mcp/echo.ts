export const description = "Echo the input back";

export const parameters = {
  type: "object",
  properties: {
    message: { type: "string", description: "Message to echo" },
  },
  required: ["message"],
};

export default async function (params: { message: string }) {
  return { echo: params.message };
}
