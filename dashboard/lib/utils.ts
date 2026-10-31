import { clsx, type ClassValue } from 'clsx';
import { twMerge } from 'tailwind-merge';

export function cn(...inputs: ClassValue[]): string {
  return twMerge(clsx(inputs));
}

export function formatLatency(microseconds: number): string {
  if (microseconds < 1000) {
    return `${microseconds}µs`;
  }
  const ms = (microseconds / 1000).toFixed(1);
  return `${ms}ms`;
}

export function formatCost(usd: number): string {
  if (usd < 0.01) {
    return `$${usd.toFixed(4)}`;
  }
  return `$${usd.toFixed(3)}`;
}

export function formatTokens(count: number): string {
  if (count >= 1_000_000) {
    return `${(count / 1_000_000).toFixed(1)}M`;
  }
  if (count >= 1_000) {
    return `${(count / 1_000).toFixed(1)}k`;
  }
  return count.toString();
}

export function truncate(str: string, maxLen = 60): string {
  if (str.length <= maxLen) return str;
  return `${str.slice(0, maxLen)}...`;
}
