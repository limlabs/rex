export const description = "Get mock weather forecast for a city";

export const parameters = {
  type: "object",
  properties: {
    city: { type: "string", description: "City name" },
    units: {
      type: "string",
      enum: ["celsius", "fahrenheit"],
      description: "Temperature units",
    },
  },
  required: ["city", "units"],
};

export default function (params: { city: string; units: "celsius" | "fahrenheit" }) {
  const temp = params.units === "celsius" ? 22 : 72;
  const symbol = params.units === "celsius" ? "C" : "F";
  return {
    city: params.city,
    temperature: `${temp}°${symbol}`,
    condition: "Sunny",
    humidity: "45%",
  };
}
