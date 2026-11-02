export interface RouteMatch {
  index: number;
  params: Record<string, string>;
}

interface RouteDef {
  pattern: RegExp;
  params: string[];
}

export const routes: RouteDef[] = [
  { pattern: /^\/$/, params: [] },
  { pattern: /^\/view\/(.+)$/, params: ["path"] },
];

export function matchRoute(pathname: string): RouteMatch {
  for (const [index, route] of routes.entries()) {
    const found = pathname.match(route.pattern);
    if (found) {
      const params: Record<string, string> = {};
      route.params.forEach((name, position) => {
        params[name] = found[position + 1]
          .split("/")
          .map(decodeURIComponent)
          .join("/");
      });
      return { index, params };
    }
  }
  return { index: 0, params: {} };
}

/** Whether the page renders `pathname` itself; other paths, such as
 * `/files/`, belong to the server. */
export function isClientRoute(pathname: string): boolean {
  return routes.some((route) => route.pattern.test(pathname));
}
