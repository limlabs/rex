// rex/auth — Server-side stub
// On the server, session data comes via getServerSideProps(context.session),
// not via the useSession hook. This stub returns a loading state.

function useSession() {
  return { data: null, status: 'loading' };
}

function signIn() {
  // No-op on server
}

function signOut() {
  // No-op on server
}

function refreshSession() {
  return Promise.resolve({});
}

exports.useSession = useSession;
exports.signIn = signIn;
exports.signOut = signOut;
exports.refreshSession = refreshSession;
