export function internalAuthorized(request: Request) {
  const expected = process.env.CHECKER_ADMIN_TOKEN;
  if (!expected) return false;
  return request.headers.get('authorization') === `Bearer ${expected}`;
}
