import React, { createContext, useContext, useState } from 'react';
import { useRouter } from 'rex/router';

const Ctx = createContext('provided');

// Regression page for dev SSR (limlabs/rex#240): hooks must run on the React
// instance the server renderer dispatches through, and useRouter() must report
// this page's route on the server as it does on the client.
export default function Hooks() {
  const [count] = useState(1);
  const value = useContext(Ctx);
  const router = useRouter();
  return (
    <div>
      <h1>Hooks</h1>
      <p>{`count:${count}`}</p>
      <p>{`context:${value}`}</p>
      <p>{`pathname:${router.pathname}`}</p>
    </div>
  );
}
