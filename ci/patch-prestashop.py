#!/usr/bin/env python3
"""
PrestaShop Portable Build-time Core Patcher (Python fallback)
Patches PrestaShop core files to fix installation, OpenSSL RSA key generation, and module caching issues.
"""

import os
import sys

def main():
    if len(sys.argv) < 2:
        print("Usage: python3 ci/patch-prestashop.py <path-to-app-dir>")
        sys.exit(1)

    app_dir = os.path.abspath(sys.argv[1].rstrip("/\\"))
    if not os.path.isdir(app_dir):
        print(f"Error: {app_dir} is not a directory")
        sys.exit(1)

    print(f"Patching PrestaShop core in: {app_dir}")

    patches = [
        # 1. install/init.php
        {
            "file": os.path.join(app_dir, "install", "init.php"),
            "find": "require_once 'install_version.php';",
            "replace": "require_once 'install_version.php';\n\nif (!defined('PS_INSTALLATION_IN_PROGRESS')) {\n    define('PS_INSTALLATION_IN_PROGRESS', true);\n}",
            "check": "if (!defined('PS_INSTALLATION_IN_PROGRESS')) {",
        },
        {
            "file": os.path.join(app_dir, "install", "init.php"),
            "find": "define('PS_INSTALLATION_IN_PROGRESS', true);",
            "replace": "if (!defined('PS_INSTALLATION_IN_PROGRESS')) {\n    define('PS_INSTALLATION_IN_PROGRESS', true);\n}",
            "check": "if (!defined('PS_INSTALLATION_IN_PROGRESS')) {\n    define('PS_INSTALLATION_IN_PROGRESS', true);\n}",
        },
        # 2. app/AppKernel.php
        {
            "file": os.path.join(app_dir, "app", "AppKernel.php"),
            "find": "$this->getEnvironment() === 'test'",
            "replace": "$this->getEnvironment() === 'test' || defined('PS_INSTALLATION_IN_PROGRESS')",
            "check": "defined('PS_INSTALLATION_IN_PROGRESS')",
        },
        # 3. src/Adapter/Container/ContainerParametersExtension.php
        {
            "file": os.path.join(app_dir, "src", "Adapter", "Container", "ContainerParametersExtension.php"),
            "find": "$this->environment->getName() === 'test'",
            "replace": "$this->environment->getName() === 'test' || defined('PS_INSTALLATION_IN_PROGRESS')",
            "check": "defined('PS_INSTALLATION_IN_PROGRESS')",
        },
        # 4. src/Adapter/ContainerBuilder.php
        {
            "file": os.path.join(app_dir, "src", "Adapter", "ContainerBuilder.php"),
            "find": "$this->environment->getName() === 'test'",
            "replace": "$this->environment->getName() === 'test' || defined('PS_INSTALLATION_IN_PROGRESS')",
            "check": "defined('PS_INSTALLATION_IN_PROGRESS')",
        },
        # 5. src/Adapter/Module/Repository/CachedModuleRepository.php
        {
            "file": os.path.join(app_dir, "src", "Adapter", "Module", "Repository", "CachedModuleRepository.php"),
            "find": "    public function getInstalledModules(): array\n    {\n        return $this->cache->get('installed_modules', function () {\n            return $this->decorated->getInstalledModules();\n        });\n    }\n\n    public function getPresentModules(): array\n    {\n        return $this->cache->get('present_modules', function () {\n            return $this->decorated->getPresentModules();\n        });\n    }\n\n    public function getActiveModules(): array\n    {\n        return $this->cache->get('active_modules', function () {\n            return $this->decorated->getActiveModules();\n        });\n    }",
            "replace": "    public function getInstalledModules(): array\n    {\n        if (defined('PS_INSTALLATION_IN_PROGRESS')) {\n            return $this->decorated->getInstalledModules();\n        }\n\n        $installed = $this->cache->get('installed_modules', function () {\n            return $this->decorated->getInstalledModules();\n        });\n\n        if (empty($installed)) {\n            $fresh = $this->decorated->getInstalledModules();\n            if (!empty($fresh)) {\n                $this->cache->delete('installed_modules');\n\n                return $this->cache->get('installed_modules', function () use ($fresh) {\n                    return $fresh;\n                });\n            }\n        }\n\n        return $installed;\n    }\n\n    public function getPresentModules(): array\n    {\n        if (defined('PS_INSTALLATION_IN_PROGRESS')) {\n            return $this->decorated->getPresentModules();\n        }\n\n        return $this->cache->get('present_modules', function () {\n            return $this->decorated->getPresentModules();\n        });\n    }\n\n    public function getActiveModules(): array\n    {\n        if (defined('PS_INSTALLATION_IN_PROGRESS')) {\n            return $this->decorated->getActiveModules();\n        }\n\n        return $this->cache->get('active_modules', function () {\n            return $this->decorated->getActiveModules();\n        });\n    }\n\n    public function clearCache(): bool\n    {\n        if ($this->cache instanceof \\Symfony\\Component\\Cache\\Adapter\\AdapterInterface) {\n            return $this->cache->clear();\n        }\n\n        return true;\n    }",
            "check": "public function clearCache(): bool",
        },
        # 6. install/controllers/http/process.php
        {
            "file": os.path.join(app_dir, "install", "controllers", "http", "process.php"),
            "find": "} catch (\\Exception $e) {",
            "replace": "} catch (\\Throwable $e) {",
            "check": "catch (\\Throwable $e)",
        },
        {
            "file": os.path.join(app_dir, "install", "controllers", "http", "process.php"),
            "find": "$this->session->process_validated = array_merge($this->session->process_validated, ['installModules' => true]);\n        $this->ajaxJsonAnswer(true);",
            "replace": "$this->session->process_validated = array_merge($this->session->process_validated, ['installModules' => true]);\n\n        try {\n            $adminModulesCache = _PS_ROOT_DIR_ . '/var/cache/' . _PS_ENV_ . '/admin/modules';\n            if (is_dir($adminModulesCache)) {\n                (new \\Symfony\\Component\\Filesystem\\Filesystem())->remove($adminModulesCache);\n            }\n        } catch (\\Throwable) {\n        }\n\n        $this->ajaxJsonAnswer(true);",
            "check": "$adminModulesCache = _PS_ROOT_DIR_",
        },
        # 7. src/PrestaShopBundle/Install/Install.php
        {
            "file": os.path.join(app_dir, "src", "PrestaShopBundle", "Install", "Install.php"),
            "find": "        $key = PhpEncryption::createNewRandomKey();\n        $privateKey = openssl_pkey_new([\n            'private_key_bits' => 2048,\n            'private_key_type' => OPENSSL_KEYTYPE_RSA,\n        ]);\n        openssl_pkey_export($privateKey, $apiPrivateKey);\n        $apiPublicKey = openssl_pkey_get_details($privateKey)['key'];",
            "replace": "        $key = PhpEncryption::createNewRandomKey();\n        $openSslConfig = [\n            'private_key_bits' => 2048,\n            'private_key_type' => OPENSSL_KEYTYPE_RSA,\n        ];\n        $openSslCnf = _PS_ROOT_DIR_ . '/config/openssl.cnf';\n        if (!file_exists($openSslCnf)) {\n            $openSslCnf = dirname(_PS_ROOT_DIR_) . '/config/openssl.cnf';\n        }\n        if (file_exists($openSslCnf)) {\n            $openSslConfig['config'] = $openSslCnf;\n        }\n\n        $privateKey = openssl_pkey_new($openSslConfig);\n        if ($privateKey !== false) {\n            openssl_pkey_export($privateKey, $apiPrivateKey, null, !empty($openSslConfig['config']) ? ['config' => $openSslConfig['config']] : null);\n            $apiPublicKey = ($details = openssl_pkey_get_details($privateKey)) ? $details['key'] : '';\n        } else {\n            $apiPrivateKey = '';\n            $apiPublicKey = '';\n        }",
            "check": "$openSslConfig = [",
        },
    ]

    patched_count = 0
    for p in patches:
        filepath = p["file"]
        if not os.path.isfile(filepath):
            continue

        with open(filepath, "r", encoding="utf-8", errors="replace") as f:
            content = f.read()

        if p["check"] in content:
            continue

        normalized = content.replace("\r\n", "\n")
        if p["find"] in normalized:
            new_content = normalized.replace(p["find"], p["replace"])
            if "\r\n" in content:
                new_content = new_content.replace("\n", "\r\n")
            with open(filepath, "w", encoding="utf-8") as f:
                f.write(new_content)
            patched_count += 1
            print(f"Patched: {os.path.basename(filepath)}")

    print(f"Successfully applied {patched_count} patch(es).")

if __name__ == "__main__":
    main()
