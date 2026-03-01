export const description = "Echo back the provided message";

export const parameters = {
  type: "object",
  properties: {
    message: { type: "string", description: "Message to echo" },
  },
  required: ["message"],
};

export default function (params: { message: string }) {
  return { echo: params.message };
}
