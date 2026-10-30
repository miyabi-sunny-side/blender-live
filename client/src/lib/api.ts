export interface Item {
  id: string;
  name: string;
  summary: string;
  status: string;
  updated_at: string;
  body: string;
}

async function requestJson<T>(url: string, signal?: AbortSignal): Promise<T> {
  const response = await fetch(url, { signal });
  if (!response.ok) {
    throw new Error(`HTTP ${response.status}`);
  }
  return (await response.json()) as T;
}

export function fetchItems(signal?: AbortSignal): Promise<Item[]> {
  return requestJson("/api/items", signal);
}

export function fetchItem(id: string, signal?: AbortSignal): Promise<Item> {
  return requestJson(`/api/items/${encodeURIComponent(id)}`, signal);
}
