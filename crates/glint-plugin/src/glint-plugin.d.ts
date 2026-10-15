/** Glint plugin API v1. No Node, filesystem, process or network APIs are implicit. */
declare const glint: {
  readonly apiVersion: 1;
  on(name: string, callback: (payload: any) => unknown): void;
  readonly host: { call(name: string, ...args: unknown[]): any };
};
