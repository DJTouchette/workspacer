import { execFileSync } from 'node:child_process';
import path from 'node:path';

/** This source is bundled into Electron and the standalone headless .cjs. It
 * targets Windows PowerShell 5.1's built-in .NET Framework; no installed module,
 * downloaded executable, writable helper script, or renderer code is executed.
 */
export const INTENT_WINDOWS_NATIVE_SOURCE = String.raw`
using System;
using System.IO;
using System.Text;
using System.Text.RegularExpressions;
using System.Collections.Generic;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Security.Cryptography;
using Microsoft.Win32.SafeHandles;

public static class IntentNativeFiles {
  [StructLayout(LayoutKind.Sequential)] struct US { public ushort Length, MaximumLength; public IntPtr Buffer; }
  [StructLayout(LayoutKind.Sequential)] struct OA { public int Length; public IntPtr RootDirectory, ObjectName; public uint Attributes; public IntPtr SecurityDescriptor, SecurityQualityOfService; }
  [StructLayout(LayoutKind.Sequential)] struct IOSB { public IntPtr Status, Information; }
  [StructLayout(LayoutKind.Sequential)] struct Info { public uint Attributes; public System.Runtime.InteropServices.ComTypes.FILETIME Creation, Access, Write; public uint Volume, SizeHigh, SizeLow, Links, IdHigh, IdLow; }
  [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)] static extern SafeFileHandle CreateFileW(string name,uint access,uint share,IntPtr security,uint disposition,uint flags,IntPtr template);
  [DllImport("kernel32.dll", SetLastError=true)] static extern bool GetFileInformationByHandle(SafeFileHandle handle,out Info info);
  [DllImport("kernel32.dll", SetLastError=true)] static extern bool SetFileInformationByHandle(SafeFileHandle handle,int kind,ref int info,uint size);
  [DllImport("ntdll.dll")] static extern int NtCreateFile(out SafeFileHandle handle,uint access,ref OA attributes,out IOSB io,IntPtr allocation,uint fileAttributes,uint share,uint disposition,uint options,IntPtr ea,uint eaLength);
  [DllImport("ntdll.dll")] static extern uint RtlNtStatusToDosError(int status);
  const uint Read = 0x80000000, Write = 0x40000000, Delete = 0x10000, Sync = 0x100000;
  static void Error() { throw new Win32Exception(Marshal.GetLastWin32Error()); }
  static void Leaf(string name) {
    if (String.IsNullOrEmpty(name) || name=="." || name==".." || name.Length>255 || Regex.IsMatch(name,@"[<>:""/\\|?*\x00-\x1f]") || name.EndsWith(".") || name.EndsWith(" ") || Regex.IsMatch(name,@"^(CON|PRN|AUX|NUL|CLOCK\$|CONIN\$|CONOUT\$|COM[1-9¹²³]|LPT[1-9¹²³])(\..*)?$",RegexOptions.IgnoreCase)) throw new IOException("Invalid Windows artifact path component");
  }
  static string DirectoryName(string directory,out string root,out string[] parts) {
    if (directory==null || directory.Length>30000) throw new IOException("Invalid Windows artifact directory");
    string value=directory.Replace('/','\\');
    if (value.StartsWith(@"\\?\") || value.StartsWith(@"\\.\") || value.StartsWith(@"\??\")) throw new IOException("Windows device namespaces are not allowed");
    if (Regex.IsMatch(value,@"^[A-Za-z]:\\")) { root=value.Substring(0,3); parts=value.Substring(3).TrimEnd('\\').Split(new char[]{'\\'},StringSplitOptions.RemoveEmptyEntries); }
    else if (value.StartsWith(@"\\")) {
      string[] all=value.Substring(2).Split('\\');
      if (all.Length<2 || all[0].Length==0 || all[1].Length==0) throw new IOException("A complete UNC server and share are required");
      Leaf(all[0]); Leaf(all[1]); root=@"\\"+all[0]+@"\"+all[1]+@"\";
      parts=value.Substring(root.Length-1).Trim('\\').Split(new char[]{'\\'},StringSplitOptions.RemoveEmptyEntries);
    } else throw new IOException("An absolute drive or UNC directory is required");
    foreach(string part in parts) Leaf(part);
    if (value.Contains(@"\\") && !value.StartsWith(@"\\")) throw new IOException("Empty Windows path components are not allowed");
    return root+String.Join(@"\",parts);
  }
  static void Ordinary(SafeFileHandle handle,bool directory) {
    Info info; if(!GetFileInformationByHandle(handle,out info)) Error();
    if ((info.Attributes & 0x400)!=0 || ((info.Attributes & 0x10)!=0)!=directory || (!directory && info.Links!=1)) throw new IOException("Reparse points, junctions, and nonordinary artifact files are not allowed");
  }
  static SafeFileHandle Relative(SafeFileHandle parent,string name,bool directory,uint access,uint disposition,uint share) {
    Leaf(name); IntPtr chars=Marshal.StringToHGlobalUni(name), unicode=IntPtr.Zero;
    try {
      US us=new US { Length=(ushort)(name.Length*2),MaximumLength=(ushort)(name.Length*2+2),Buffer=chars };
      unicode=Marshal.AllocHGlobal(Marshal.SizeOf(typeof(US))); Marshal.StructureToPtr(us,unicode,false);
      OA oa=new OA { Length=Marshal.SizeOf(typeof(OA)),RootDirectory=parent.DangerousGetHandle(),ObjectName=unicode,Attributes=0x40|0x1000 }; // CASE_INSENSITIVE | DONT_REPARSE
      IOSB io; SafeFileHandle file;
      int status=NtCreateFile(out file,access|Sync,ref oa,out io,IntPtr.Zero,0,share,disposition,0x20|0x200000|(directory?1u:0x40u),IntPtr.Zero,0);
      if(status<0) { if(file!=null) file.Dispose(); throw new Win32Exception((int)RtlNtStatusToDosError(status)); }
      try { Ordinary(file,directory); return file; } catch { file.Dispose(); throw; }
    } finally { if(unicode!=IntPtr.Zero)Marshal.FreeHGlobal(unicode); Marshal.FreeHGlobal(chars); }
  }
  sealed class DirectoryLease:IDisposable {
    public List<SafeFileHandle> Handles=new List<SafeFileHandle>();
    public SafeFileHandle Last { get { return Handles[Handles.Count-1]; } }
    public DirectoryLease(string directory,bool create) {
      string root; string[] parts; DirectoryName(directory,out root,out parts);
      string extended=root.StartsWith(@"\\") ? @"\\?\UNC\"+root.Substring(2) : @"\\?\"+root;
      try {
        SafeFileHandle first=CreateFileW(extended,0xa0,1,IntPtr.Zero,3,0x2000000|0x200000,IntPtr.Zero);
        if(first.IsInvalid) { first.Dispose(); Error(); }
        Handles.Add(first); Ordinary(first,true);
        // NtCreateFile resolves each single component relative to its pinned
        // parent. Denying WRITE/DELETE sharing also prevents junction mutation
        // and rename while the lease is held, including existing parents.
        foreach(string part in parts) Handles.Add(Relative(Last,part,true,0xa0,create?3u:1u,1));
      } catch { Dispose(); throw; }
    }
    public void Dispose() { for(int i=Handles.Count-1;i>=0;i--) Handles[i].Dispose(); Handles.Clear(); }
  }
  static byte[] Bytes(FileStream stream,int limit) {
    if(limit<0 || limit>16*1024*1024 || stream.Length>limit) throw new IOException("Artifact exceeds its byte limit");
    byte[] buffer=new byte[limit+1]; int count=0,n;
    while(count<buffer.Length && (n=stream.Read(buffer,count,buffer.Length-count))>0) count+=n;
    if(count>limit) throw new IOException("Artifact exceeds its byte limit");
    byte[] result=new byte[count]; Buffer.BlockCopy(buffer,0,result,0,count); return result;
  }
  static string Hash(byte[] bytes) { using(SHA256 sha=SHA256.Create()) return BitConverter.ToString(sha.ComputeHash(bytes)).Replace("-","").ToLowerInvariant(); }
  public static string[] ReadMany(string directory,string[] leaves,int limit) {
    if(leaves==null || leaves.Length>256 || (long)limit*leaves.Length>32*1024*1024) throw new IOException("Artifact batch exceeds its limit");
    using(DirectoryLease lease=new DirectoryLease(directory,false)) {
      string[] values=new string[leaves.Length];
      for(int i=0;i<leaves.Length;i++) using(SafeFileHandle file=Relative(lease.Last,leaves[i],false,Read,1,1)) using(FileStream stream=new FileStream(file,FileAccess.Read)) values[i]=Convert.ToBase64String(Bytes(stream,limit));
      return values;
    }
  }
  public static void WriteFile(string directory,string leaf,string data,string expected,int limit) {
    byte[] bytes=Convert.FromBase64String(data); if(bytes.Length>limit || limit<0 || limit>16*1024*1024)throw new IOException("Artifact exceeds its byte limit");
    using(DirectoryLease lease=new DirectoryLease(directory,true)) {
      // FILE_CREATE never follows or overwrites an existing leaf. Replacements
      // lock the existing ordinary file exclusively, check its exact bytes,
      // and write through that same handle. A failure remains an unknown receipt.
      using(SafeFileHandle file=Relative(lease.Last,leaf,false,Read|Write,expected==null?2u:1u,0))
      using(FileStream stream=new FileStream(file,FileAccess.ReadWrite)) {
        if(expected!=null && Hash(Bytes(stream,limit))!=expected)throw new IOException("Project document changed during promotion");
        stream.Position=0; stream.Write(bytes,0,bytes.Length); stream.SetLength(bytes.Length); stream.Flush(true);
      }
    }
  }
  public static void RemoveFile(string directory,string leaf,string expected,int limit) {
    using(DirectoryLease lease=new DirectoryLease(directory,false))
    using(SafeFileHandle file=Relative(lease.Last,leaf,false,Read|Delete,1,0))
    using(FileStream stream=new FileStream(file,FileAccess.Read)) {
      if(Hash(Bytes(stream,limit))!=expected) throw new IOException("Artifact changed before cleanup");
      int remove=1; if(!SetFileInformationByHandle(file,4,ref remove,4))Error();
    }
  }
}
`;

const SCRIPT = `$ErrorActionPreference='Stop'; [Console]::InputEncoding=New-Object System.Text.UTF8Encoding($false); [Console]::OutputEncoding=New-Object System.Text.UTF8Encoding($false); try { Add-Type -TypeDefinition @'\n${INTENT_WINDOWS_NATIVE_SOURCE}\n'@; $r=([Console]::In.ReadToEnd() | ConvertFrom-Json); if ($r.action -eq 'read') { $values=[IntentNativeFiles]::ReadMany([string]$r.directory,[string[]]$r.leaves,[int]$r.limit); @{ok=$true;values=@($values)} | ConvertTo-Json -Compress -Depth 4 } elseif ($r.action -eq 'write') { $expected=$null; if($null -ne $r.expected) {$expected=[string]$r.expected}; [IntentNativeFiles]::WriteFile([string]$r.directory,[string]$r.leaf,[string]$r.data,$expected,[int]$r.limit); '{"ok":true}' } elseif ($r.action -eq 'remove') { [IntentNativeFiles]::RemoveFile([string]$r.directory,[string]$r.leaf,[string]$r.expected,[int]$r.limit); '{"ok":true}' } else {throw 'Unknown secure file operation'} } catch { @{ok=$false;error=$_.Exception.Message} | ConvertTo-Json -Compress; exit 1 }`;

export interface IntentWindowsFileRequest {
  action: 'read' | 'write' | 'remove';
  directory: string;
  leaves?: string[];
  leaf?: string;
  data?: string;
  expected?: string | null;
  limit: number;
}
export function runIntentWindowsFiles(request: IntentWindowsFileRequest): { values?: string[] } {
  if (process.platform !== 'win32') throw new Error('Windows file helper requires Windows');
  const executable = path.win32.join(
    process.env.SystemRoot || 'C:\\Windows',
    'System32',
    'WindowsPowerShell',
    'v1.0',
    'powershell.exe',
  );
  let stdout: string;
  try {
    stdout = execFileSync(
      executable,
      ['-NoLogo', '-NoProfile', '-NonInteractive', '-Command', SCRIPT],
      {
        input: JSON.stringify(request),
        encoding: 'utf8',
        windowsHide: true,
        timeout: 30_000,
        maxBuffer: 48 * 1024 * 1024,
      },
    );
  } catch (error) {
    const output = (error as { stdout?: string }).stdout;
    if (output) {
      try {
        const response = JSON.parse(output);
        if (response.error) throw new Error(String(response.error));
      } catch (parsed) {
        if (parsed instanceof Error && !(parsed instanceof SyntaxError)) throw parsed;
      }
    }
    throw new Error('Secure Windows file operation failed; Windows PowerShell must be available');
  }
  const response = JSON.parse(stdout);
  if (
    response.ok !== true ||
    (request.action === 'read' &&
      (!Array.isArray(response.values) || response.values.length !== request.leaves?.length))
  )
    throw new Error('Invalid secure Windows file response');
  return response;
}
